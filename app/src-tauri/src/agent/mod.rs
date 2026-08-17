//! 应用内 agent：读账本、解读数据、按对话改界面布局。
//!
//! 分层：
//! - `api`      —— Claude Messages API 的最小 HTTP 客户端（流式 + 工具调用）
//! - `tools`    —— 工具面。安全姿态靠「没有」保证：没有审批、没有写账
//! - `catalog`  —— 前端注册表推过来的块白名单，agent 改布局的合法性边界
//! - `session`  —— 系统提示、对话历史、工具循环
//!
//! **模型调用只发生在 Rust 侧。** API key 不进 webview，页面里任何东西
//! （包括 agent 自己生成的布局）都拿不到它。

pub mod api;
pub mod catalog;
pub mod session;
pub mod tools;

pub use api::Effort;
pub use catalog::BlockCatalog;
pub use session::{AgentSession, EVENT_DONE};
