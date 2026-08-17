//! agent 的工具面。
//!
//! 工具表的安全姿态和 `taxmcp` 一样，**靠「没有」来保证**：
//!
//! - 没有 `approve_entry` / `reject_entry`。把草稿变成账是人的决定，
//!   agent 连表述这个动作的词汇都不该有。
//! - 没有 `ingest_document`、没有 `set_document_status`。收文档和忽略文档
//!   都有人的入口，不给 agent 代劳。
//! - 读操作全开——这与 ROADMAP 里 MCP 的读写分界一致。
//!
//! 唯一的写操作是 `apply_layout`，它改的是**呈现**，不是账。而且落库前
//! 必过 `BlockCatalog` 校验：模型能产出任意 JSON，但只有命中白名单的才写得进去。

use std::sync::Mutex;

use serde_json::{Value, json};

use crate::agent::catalog::BlockCatalog;
use crate::backend::Backend;
use crate::layout::LayoutStore;

pub struct ToolDeps<'a> {
    pub backend: &'a Mutex<Box<dyn Backend>>,
    pub layout: &'a Mutex<LayoutStore>,
    pub catalog: &'a BlockCatalog,
}

/// 一次工具调用的结果。`layout_changed` 让上层知道要不要通知前端重载。
pub struct ToolOutcome {
    pub content: String,
    pub is_error: bool,
    pub layout_changed: bool,
}

impl ToolOutcome {
    fn ok(value: Value) -> Self {
        ToolOutcome {
            content: value.to_string(),
            is_error: false,
            layout_changed: false,
        }
    }

    fn err(message: impl Into<String>) -> Self {
        ToolOutcome {
            content: message.into(),
            is_error: true,
            layout_changed: false,
        }
    }
}

/// 工具定义，原样进请求体。顺序固定——tools 参与 prompt cache 前缀，
/// 每次重排都会让缓存失效。
pub fn definitions() -> Value {
    json!([
        {
            "name": "get_overview",
            "description": "读取账本总览：待审草稿、待处理文档、已入账分录、未对账银行流水、账户表。\
                            要回答「现在有什么待办」「账上有多少笔」这类问题时先调这个。",
            "input_schema": { "type": "object", "properties": {}, "additionalProperties": false }
        },
        {
            "name": "get_gst_return",
            "description": "生成某个申报期的 GST101 各行，每行带 provenance（背后是哪些分录）。\
                            date 落在哪个申报期由引擎判断。",
            "input_schema": {
                "type": "object",
                "properties": {
                    "date": { "type": "string", "description": "申报期内任一天，YYYY-MM-DD；缺省为今天" },
                    "frequency": { "type": "string", "description": "申报频率 id，如 two_monthly；缺省用规则文件的默认值" }
                },
                "additionalProperties": false
            }
        },
        {
            "name": "get_ir3_summary",
            "description": "生成某个税年的 IR3 汇总（收入、支出、净利润与所得税分档），每行带 provenance。",
            "input_schema": {
                "type": "object",
                "properties": {
                    "year": { "type": "string", "description": "税年标签，如 2025-26" }
                },
                "required": ["year"],
                "additionalProperties": false
            }
        },
        {
            "name": "get_document",
            "description": "读一份文档的元信息和记在它身上的所有 extraction（含校验问题）。",
            "input_schema": {
                "type": "object",
                "properties": {
                    "document_id": { "type": "string" }
                },
                "required": ["document_id"],
                "additionalProperties": false
            }
        },
        {
            "name": "get_layout",
            "description": "读当前的布局文档。改布局之前必须先读——你要在现有文档上改，\
                            而不是凭空造一份新的。",
            "input_schema": { "type": "object", "properties": {}, "additionalProperties": false }
        },
        {
            "name": "list_block_types",
            "description": "列出所有可用的块型、它们的类别、可配置的 props 和文案键，\
                            以及 binding 允许指向的数据源。布局里只能用这里列出的块型。",
            "input_schema": { "type": "object", "properties": {}, "additionalProperties": false }
        },
        {
            "name": "apply_layout",
            "description": "写入一份完整的新布局文档（不是增量补丁——把 get_layout 读到的文档改好后整份传回来）。\
                            写入前会校验：块型必须已注册、binding 的数据源必须合法、span 必须在栅格范围内、\
                            人工确认闸口不能被删。校验不过会把所有问题一次性告诉你，改完再试。\
                            用户随时可以撤销你的改动。",
            "input_schema": {
                "type": "object",
                "properties": {
                    "doc": { "type": "object", "description": "完整的布局文档，形如 { version: 1, pages: [...] }" },
                    "summary": { "type": "string", "description": "一句话说明这次改了什么，给用户看" }
                },
                "required": ["doc", "summary"],
                "additionalProperties": false
            }
        }
    ])
}

pub fn dispatch(name: &str, input: &Value, deps: &ToolDeps<'_>) -> ToolOutcome {
    match name {
        "get_overview" => backend_call(deps, |b| b.overview()),

        "get_gst_return" => {
            let date = string_arg(input, "date");
            let frequency = string_arg(input, "frequency");
            backend_call(deps, |b| b.gst(date.clone(), frequency.clone()))
        }

        "get_ir3_summary" => match string_arg(input, "year") {
            Some(year) => backend_call(deps, |b| b.ir3(year.clone())),
            None => ToolOutcome::err("缺少参数 year"),
        },

        "get_document" => match string_arg(input, "document_id") {
            Some(id) => backend_call(deps, |b| b.document(id.clone())),
            None => ToolOutcome::err("缺少参数 document_id"),
        },

        "get_layout" => match deps.layout.lock() {
            Ok(store) => match store.load() {
                Ok(Some(doc)) => ToolOutcome::ok(doc),
                // 前端启动时会把当前文档存一份，所以正常不会走到这里。
                Ok(None) => ToolOutcome::err("布局还没有初始化，请让用户先打开一次界面"),
                Err(e) => ToolOutcome::err(e),
            },
            Err(_) => ToolOutcome::err("内部状态已损坏"),
        },

        "list_block_types" => ToolOutcome::ok(json!({
            "blocks": deps.catalog.blocks,
            "sources": deps.catalog.sources,
            "grid": { "desktop_columns": 12, "mobile_columns": 4 },
        })),

        "apply_layout" => apply_layout(input, deps),

        // 明确区分「这个工具不存在」和「你参数写错了」——
        // 尤其要让模型知道审批类动作不是它能做的。
        "approve_entry" | "reject_entry" | "post_entry" => ToolOutcome::err(
            "这个动作不存在于你的工具表。批准或拒绝草稿是人的决定，只能由用户在界面上点击完成。\
             你可以把需要确认的内容整理出来给用户看。",
        ),

        other => ToolOutcome::err(format!("未知工具 {other}")),
    }
}

fn apply_layout(input: &Value, deps: &ToolDeps<'_>) -> ToolOutcome {
    let Some(doc) = input.get("doc") else {
        return ToolOutcome::err("缺少参数 doc");
    };

    if let Err(problems) = deps.catalog.validate_doc(doc) {
        // 一次把所有问题说完，模型才能一轮改对，而不是挤牙膏式来回。
        return ToolOutcome::err(format!(
            "布局校验不通过，没有写入。请修正后重试：\n- {}",
            problems.join("\n- ")
        ));
    }

    let mut store = match deps.layout.lock() {
        Ok(store) => store,
        Err(_) => return ToolOutcome::err("内部状态已损坏"),
    };

    match store.save(doc) {
        Ok(version) => ToolOutcome {
            content: json!({ "ok": true, "version": version }).to_string(),
            is_error: false,
            layout_changed: true,
        },
        Err(e) => ToolOutcome::err(e),
    }
}

fn backend_call(
    deps: &ToolDeps<'_>,
    call: impl FnOnce(&mut Box<dyn Backend>) -> Result<Value, String>,
) -> ToolOutcome {
    // 锁在这个作用域里拿了就还，不跨 await——上层是异步的，跨 await 持有
    // std::sync 的锁会把整个 runtime 卡住。
    let mut guard = match deps.backend.lock() {
        Ok(guard) => guard,
        Err(_) => return ToolOutcome::err("内部状态已损坏"),
    };
    match call(&mut guard) {
        Ok(value) => ToolOutcome::ok(value),
        Err(e) => ToolOutcome::err(e),
    }
}

fn string_arg(input: &Value, key: &str) -> Option<String> {
    input
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tool_table_has_no_way_to_approve_anything() {
        let text = definitions().to_string();
        for forbidden in ["approve", "reject", "post_entry", "ingest"] {
            assert!(
                !text.contains(forbidden),
                "工具表里不该出现 {forbidden}：人工闸口和写操作不归 agent"
            );
        }
    }

    #[test]
    fn asking_to_approve_gets_an_explanation_not_a_bare_unknown_tool() {
        let catalog = BlockCatalog::default();
        let store = Mutex::new(LayoutStore::open(&std::env::temp_dir()).unwrap());
        let backend: Mutex<Box<dyn Backend>> = Mutex::new(Box::new(
            crate::backend::UnavailableBackend::new("测试"),
        ));
        let deps = ToolDeps {
            backend: &backend,
            layout: &store,
            catalog: &catalog,
        };

        let out = dispatch("approve_entry", &json!({}), &deps);
        assert!(out.is_error);
        assert!(out.content.contains("人的决定"), "{}", out.content);
    }

    #[test]
    fn every_tool_declares_a_schema_that_forbids_extra_arguments() {
        let defs = definitions();
        for tool in defs.as_array().unwrap() {
            assert_eq!(
                tool["input_schema"]["additionalProperties"],
                json!(false),
                "{} 的 schema 应当拒绝多余参数",
                tool["name"]
            );
        }
    }
}
