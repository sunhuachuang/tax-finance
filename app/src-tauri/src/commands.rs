//! `#[tauri::command]` 层。刻意做薄：解析参数 → 转给 backend / layout store → 回 JSON。
//!
//! 这里不该出现任何业务逻辑或算术。所有计算都在 `service/` 的库里，
//! 「模型报告数字，从不计算数字」的另一面是「UI 层也不计算数字」。

use std::sync::Mutex;

use serde_json::Value;
use tauri::{Emitter as _, State};

use crate::agent::{AgentSession, BlockCatalog, EVENT_DONE, Effort};
use crate::backend::Backend;
use crate::layout::LayoutStore;
use crate::settings::Settings;

pub struct AppState {
    pub backend: Mutex<Box<dyn Backend>>,
    pub layout: Mutex<LayoutStore>,
    /// 数据来源的一行描述，状态栏展示用。
    pub source: String,

    /// 对话历史活在这里，不在 webview 里——API key 也一样。
    /// 用 tokio 的锁：会话要跨 await 持有。
    pub agent: tokio::sync::Mutex<Option<AgentSession>>,
    /// 前端组件注册表的镜像，agent 改布局时的白名单。
    pub catalog: Mutex<BlockCatalog>,

    pub settings: Mutex<Settings>,
    /// app 数据目录：settings.json 和 ui.db 都在这里，不进账本目录。
    pub app_dir: std::path::PathBuf,
}

/// 锁被 poison 说明上一次调用 panic 了。与其继续用一个状态不明的
/// `Store`，不如让这次调用明确失败。
fn locked<'a, T>(lock: &'a Mutex<T>) -> Result<std::sync::MutexGuard<'a, T>, String> {
    lock.lock()
        .map_err(|_| "内部状态已损坏，请重启应用".to_string())
}

/// 命令的错误只会出现在 UI 的某个角落里，排错时未必看得到。
/// 这里统一往 stderr 记一条，让 `tauri dev` 的终端能直接看出哪个命令失败了。
fn logged<T>(command: &str, result: Result<T, String>) -> Result<T, String> {
    if let Err(message) = &result {
        eprintln!("financeapp: {command} 失败：{message}");
    }
    result
}

/// 前端启动时第一个调的命令，因此也拿来当「webview 活着」的探针：
/// 白屏最常见的原因是 CSP 把脚本挡了，那种情况下这行日志不会出现。
#[tauri::command]
pub fn data_source(state: State<'_, AppState>) -> String {
    eprintln!("financeapp: 前端已连接");
    state.source.clone()
}

#[tauri::command]
pub fn overview(state: State<'_, AppState>) -> Result<Value, String> {
    logged("overview", locked(&state.backend)?.overview())
}

#[tauri::command]
pub fn gst(
    state: State<'_, AppState>,
    date: Option<String>,
    frequency: Option<String>,
) -> Result<Value, String> {
    logged("gst", locked(&state.backend)?.gst(date, frequency))
}

#[tauri::command]
pub fn ir3(state: State<'_, AppState>, year: String) -> Result<Value, String> {
    logged("ir3", locked(&state.backend)?.ir3(year))
}

/// 收一份文档。只造 pending 记录，不产生账——和 MCP 的 `ingest_document` 同一层级。
#[tauri::command]
pub fn ingest_document(state: State<'_, AppState>, path: String) -> Result<Value, String> {
    logged("ingest_document", locked(&state.backend)?.ingest_document(path))
}

#[tauri::command]
pub fn document(state: State<'_, AppState>, document_id: String) -> Result<Value, String> {
    logged("document", locked(&state.backend)?.document(document_id))
}

/// 人工决定：忽略 / 放回待提取。不声明「已提取」——那要读过文档才能说。
#[tauri::command]
pub fn set_document_status(
    state: State<'_, AppState>,
    document_id: String,
    to: String,
) -> Result<Value, String> {
    logged(
        "set_document_status",
        locked(&state.backend)?.set_document_status(document_id, to),
    )
}

/// 把文档交给系统默认程序打开。
///
/// 刻意在 Rust 侧做而不是前端调 opener 插件：capability 是给 JS→插件那条桥的，
/// 开放任意路径要配静态 scope，而数据目录是运行时才知道的。放在这里既不用开那个
/// 口子，又能先确认这条路径真的落在数据目录内——路径来自 backend，不是前端传的。
#[tauri::command]
pub fn open_document(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    document_id: String,
) -> Result<(), String> {
    let value = logged("open_document", locked(&state.backend)?.document(document_id))?;
    let path = value
        .get("local_path")
        .and_then(Value::as_str)
        .ok_or("远程模式下文件在 host 上，这台机器打不开")?;

    use tauri_plugin_opener::OpenerExt as _;
    app.opener()
        .open_path(path, None::<&str>)
        .map_err(|e| format!("打开 {path} 失败：{e}"))
}

/// 人工确认闸口。这条命令存在于 app 和 taxweb，不存在于 MCP 工具表。
#[tauri::command]
pub fn approve_entry(state: State<'_, AppState>, entry_id: String) -> Result<Value, String> {
    logged("approve_entry", locked(&state.backend)?.approve_entry(entry_id))
}

#[tauri::command]
pub fn reject_entry(state: State<'_, AppState>, entry_id: String) -> Result<Value, String> {
    logged("reject_entry", locked(&state.backend)?.reject_entry(entry_id))
}

/// 前端启动时把组件注册表推过来。
///
/// 白名单**不在 Rust 里写死**：注册表是渲染器、属性面板和 agent 共用的
/// 同一份真相，在这边再抄一份迟早会漂移，而漂移的结果是 agent 生成出
/// 渲染器认不出的块。
#[tauri::command]
pub fn set_block_catalog(state: State<'_, AppState>, catalog: BlockCatalog) -> Result<(), String> {
    *locked(&state.catalog)? = catalog;
    Ok(())
}

/// 读设置。**返回的东西里没有 key 原文**——只说有没有、从哪来、末尾几位。
/// key 在你输入的那一刻穿过 webview 一次，此后不再回来。
#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<Value, String> {
    Ok(locked(&state.settings)?.redacted())
}

/// 存 API key，并立刻启用 / 停用助手——不需要重启。
/// 传空串等于清除。
#[tauri::command]
pub async fn set_api_key(state: State<'_, AppState>, key: String) -> Result<Value, String> {
    let key = key.trim().to_string();

    let (settings, from_env) = {
        let mut guard = locked(&state.settings)?;
        guard.anthropic_api_key = if key.is_empty() { None } else { Some(key) };
        guard.save(&state.app_dir)?;
        (guard.clone(), guard.api_key_from_env())
    };

    // 环境变量优先，所以有效 key 未必是刚存的那个——按有效值重建会话。
    *state.agent.lock().await = settings.effective_api_key().map(|key| {
        AgentSession::new(key, settings.effective_base_url(), settings.effective_model())
    });

    let mut view = settings.redacted();
    if let Some(object) = view.as_object_mut() {
        // 存了却不生效（被环境变量盖住）是最容易困惑的情况，明确告诉前端。
        object.insert("api_key_from_env".into(), Value::Bool(from_env));
    }
    Ok(view)
}

/// 存模型服务地址和模型名。留空 = Anthropic 官方 + claude-opus-5。
/// 立刻生效，按新配置重建会话。
#[tauri::command]
pub async fn set_llm_endpoint(
    state: State<'_, AppState>,
    base_url: String,
    model: String,
) -> Result<Value, String> {
    let settings = {
        let mut guard = locked(&state.settings)?;
        let trimmed = |v: String| Some(v.trim().to_string()).filter(|v| !v.is_empty());
        guard.llm_base_url = trimmed(base_url);
        guard.llm_model = trimmed(model);
        guard.save(&state.app_dir)?;
        guard.clone()
    };

    *state.agent.lock().await = settings.effective_api_key().map(|key| {
        AgentSession::new(key, settings.effective_base_url(), settings.effective_model())
    });

    Ok(settings.redacted())
}

/// 存远程 host。**要重启才生效**——backend 在启动时就按模式建好了，
/// 中途换等于把整个数据来源掉包。传空串等于回到本地模式。
#[tauri::command]
pub fn set_finance_host(state: State<'_, AppState>, host: String) -> Result<Value, String> {
    let host = host.trim().to_string();
    let mut guard = locked(&state.settings)?;
    guard.finance_host = if host.is_empty() { None } else { Some(host) };
    guard.save(&state.app_dir)?;
    Ok(guard.redacted())
}

/// agent 能不能用。没有 API key 就没有 agent——前端据此决定是否显示对话入口。
#[tauri::command]
pub async fn agent_status(state: State<'_, AppState>) -> Result<Value, String> {
    let ready = state.agent.lock().await.is_some();
    Ok(serde_json::json!({
        "ready": ready,
        "reason": if ready { Value::Null } else {
            Value::String("还没有设置 Claude API Key".into())
        },
    }))
}

/// 发一句话给 agent。回复通过 `agent://*` 事件流式推给前端，
/// 这个命令只在整轮结束后返回。
#[tauri::command]
pub async fn agent_send(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    text: String,
) -> Result<(), String> {
    // 白名单先拷出来，避免把同步锁带进 await。
    let catalog = locked(&state.catalog)?.clone();

    let mut guard = state.agent.lock().await;
    let session = guard
        .as_mut()
        .ok_or("AI 助手不可用：还没有设置 Claude API Key")?;

    let result = session
        .send(&app, text, &state.backend, &state.layout, &catalog)
        .await;

    // 成功和失败都要发 done，否则界面会一直停在「思考中」。
    let _ = app.emit(
        EVENT_DONE,
        serde_json::json!({
            "ok": result.is_ok(),
            "error": result.as_ref().err(),
        }),
    );
    logged("agent_send", result)
}

/// 清空对话。布局不动——agent 改过的东西留在原地，撤销走界面上的撤销。
#[tauri::command]
pub async fn agent_reset(state: State<'_, AppState>) -> Result<(), String> {
    if let Some(session) = state.agent.lock().await.as_mut() {
        session.reset();
    }
    Ok(())
}

/// 质量 / 延迟旋钮。改布局这种小事 medium 就够，复杂分析可以调高。
#[tauri::command]
pub async fn agent_set_effort(state: State<'_, AppState>, effort: Effort) -> Result<(), String> {
    if let Some(session) = state.agent.lock().await.as_mut() {
        session.set_effort(effort);
    }
    Ok(())
}

#[tauri::command]
pub fn load_layout(state: State<'_, AppState>) -> Result<Option<Value>, String> {
    logged("load_layout", locked(&state.layout)?.load())
}

#[tauri::command]
pub fn save_layout(state: State<'_, AppState>, doc: Value) -> Result<i64, String> {
    logged("save_layout", locked(&state.layout)?.save(&doc))
}
