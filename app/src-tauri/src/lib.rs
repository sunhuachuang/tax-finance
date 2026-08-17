//! Finance 客户端的 Tauri shell。
//!
//! 这个二进制不含业务逻辑：它把 `service/` 的库 crates 装配起来，暴露成一组
//! 命令给前端。桌面直连本地账本，移动端连回常驻 host——差异被 `backend`
//! 那一层吸收，前端不感知（见 ARCHITECTURE.md）。

mod agent;
mod backend;
mod commands;
mod config;
mod layout;
mod settings;

use std::sync::Mutex;

use tauri::Manager as _;

use crate::agent::{AgentSession, BlockCatalog};
use crate::backend::{Backend, LocalBackend, RemoteBackend, UnavailableBackend};
use crate::commands::AppState;
use crate::config::Mode;
use crate::settings::Settings;
use crate::layout::LayoutStore;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // 设置和布局都存 app 自己的数据目录，不进账本目录：
            // 账本是受审计的不可变记录，这两样不是。
            let app_dir = app.path().app_data_dir()?;
            let settings = Settings::load(&app_dir);

            let mode = Mode::resolve(&settings);
            let source = mode.describe();
            let backend = build_backend(mode);

            let layout = LayoutStore::open(&app_dir).map_err(std::io::Error::other)?;

            eprintln!("financeapp: 数据来源 {source}");
            // 没有 key 不影响其余功能——界面照常用，助手面板提示去设置里填。
            let agent = settings.effective_api_key().map(|key| {
                AgentSession::new(
                    key,
                    settings.effective_base_url(),
                    settings.effective_model(),
                )
            });
            eprintln!(
                "financeapp: AI 助手{}",
                if agent.is_some() { "已启用" } else { "未启用（未设置 API key）" }
            );

            app.manage(AppState {
                backend: Mutex::new(backend),
                layout: Mutex::new(layout),
                source,
                agent: tokio::sync::Mutex::new(agent),
                catalog: Mutex::new(BlockCatalog::default()),
                settings: Mutex::new(settings),
                app_dir,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::data_source,
            commands::overview,
            commands::gst,
            commands::ir3,
            commands::ingest_document,
            commands::document,
            commands::set_document_status,
            commands::open_document,
            commands::approve_entry,
            commands::reject_entry,
            commands::load_layout,
            commands::save_layout,
            commands::set_block_catalog,
            commands::get_settings,
            commands::set_api_key,
            commands::set_finance_host,
            commands::set_llm_endpoint,
            commands::agent_status,
            commands::agent_send,
            commands::agent_reset,
            commands::agent_set_effort,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// 后端起不来不阻止应用启动——换成一个每次都报同一句原因的占位，
/// 让用户看得到到底哪里不对。
fn build_backend(mode: Mode) -> Box<dyn Backend> {
    match mode {
        Mode::Local {
            data_dir,
            rules_dir,
        } => match LocalBackend::open(&data_dir, &rules_dir) {
            Ok(backend) => Box::new(backend),
            Err(reason) => Box::new(UnavailableBackend::new(reason)),
        },
        Mode::Remote { base_url } => Box::new(RemoteBackend::new(base_url)),
    }
}
