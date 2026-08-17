//! 对话状态与工具循环。
//!
//! 对话历史存在 Rust 侧，前端只负责发消息和渲染事件——循环在这里，
//! API key 也在这里，webview 两样都拿不到。

use std::sync::Mutex;

use serde_json::json;
use tauri::{AppHandle, Emitter as _};

use crate::agent::api::{Client, Delta, Effort, Message};
use crate::agent::catalog::BlockCatalog;
use crate::agent::tools::{self, ToolDeps};
use crate::backend::Backend;
use crate::layout::LayoutStore;

/// 一轮用户消息里最多允许的工具往返。够用，且防止跑飞。
const MAX_TOOL_ROUNDS: usize = 12;

/// 前端监听的事件名。
pub const EVENT_DELTA: &str = "agent://delta";
pub const EVENT_TOOL: &str = "agent://tool";
pub const EVENT_DONE: &str = "agent://done";
pub const EVENT_LAYOUT_CHANGED: &str = "agent://layout-changed";

/// 系统提示。
///
/// 三条硬约束直接对应 ROADMAP 和 ARCHITECTURE 的架构原则；措辞按 Claude Opus 5
/// 的特性调过：明确要求简洁、明确划定范围，并且**刻意不写「请自行复核」**
/// ——这个模型本来就会自我核对，再叮嘱一遍只会让它过度验证。
const SYSTEM: &str = r#"你是这个个人财务 app 里的助手。用户是 NZ 的 sole trader，用这个 app 记账和准备报税。

你能做两件事：读账本数据并解读，以及改这个 app 的界面布局。

## 数字：报告，不计算

这是整个系统最硬的一条规则。所有算术都由确定性代码完成，你负责的是**取数、解读、提醒**。

- 任何金额、税额、笔数，都必须来自工具返回的结果，原样引用。
- **不要自己做加减乘除。** 需要一个工具没直接给出的数，就说明它没有，而不是自己算一个。
- 引用数字时说清出处（哪一期的 GST101 的哪个 box、哪个税年的 IR3 哪一行）。
- 工具返回的 provenance / contributions 是这个引擎的核心产出，用户问「这个数哪来的」时用它回答。

## 审批不归你

把草稿变成账、忽略一份文档，都是人的决定，只能由用户在界面上点。你的工具表里没有这些动作，这是有意的。你可以把需要确认的东西整理清楚给用户看，然后停在那里。

## 改布局

界面是一份 JSON 文档，你改的是它。

- 动手前先 `get_layout` 和 `list_block_types`。在现有文档上改，别凭空造新的。
- 只能用 `list_block_types` 列出的块型，binding 只能指向它列出的数据源。
- `apply_layout` 收的是**完整文档**，不是补丁。
- 数据块的值只能来自 binding。想在界面上放一个数字，就绑一个查询；不要把算出来的数写进文本块冒充数据。
- 用户随时能撤销你的改动，所以不必反复确认——按他说的改，然后说清改了什么。

## 怎么说话

先说结论。用户问什么答什么，不要把过程复述一遍。回答保持简短，把篇幅留给真正的答案而不是铺垫和免责声明。

按用户要求的范围做事。含糊的地方按一个细心同事的判断来定，只有不同理解会导致做出的东西完全不同时才反问。你觉得用户的想法有问题，就说一句，然后仍然按他要的做。"#;

pub struct AgentSession {
    client: Client,
    history: Vec<Message>,
    effort: Effort,
}

impl AgentSession {
    /// `base_url` / `model` 留空即用 Anthropic 官方和默认模型；
    /// 填了就走第三方的 Anthropic 兼容端点。
    pub fn new(api_key: String, base_url: Option<String>, model: Option<String>) -> Self {
        AgentSession {
            client: Client::new(api_key, base_url, model),
            history: Vec::new(),
            effort: Effort::default(),
        }
    }

    pub fn reset(&mut self) {
        self.history.clear();
    }

    pub fn set_effort(&mut self, effort: Effort) {
        self.effort = effort;
    }

    /// 跑完一整轮：用户说一句 → 模型思考、调工具、再思考 → 给出回复。
    pub async fn send(
        &mut self,
        app: &AppHandle,
        text: String,
        backend: &Mutex<Box<dyn Backend>>,
        layout: &Mutex<LayoutStore>,
        catalog: &BlockCatalog,
    ) -> Result<(), String> {
        self.history.push(Message::user_text(text));
        let tools = tools::definitions();

        for round in 0..MAX_TOOL_ROUNDS {
            let turn = self
                .client
                .stream_turn(SYSTEM, &tools, &self.history, self.effort, |delta| {
                    let payload = match delta {
                        Delta::Text(text) => json!({ "kind": "text", "text": text }),
                        Delta::Thinking(text) => json!({ "kind": "thinking", "text": text }),
                        Delta::ToolStarted { name } => json!({ "kind": "tool", "text": name }),
                        Delta::Notice(text) => json!({ "kind": "notice", "text": text }),
                    };
                    let _ = app.emit(EVENT_DELTA, payload);
                })
                .await?;

            // 助手这一轮必须原样入历史——thinking 块被改写过，下一轮就会被 API 拒。
            self.history.push(Message::assistant(turn.content.clone()));

            // 安全分类器可能拒答，此时是 HTTP 200 但 content 为空或不完整。
            // 不检查 stop_reason 就直接读 content，界面上会变成一次静默的空回复。
            if turn.stop_reason == "refusal" {
                return Err("Claude 拒绝了这个请求。换个问法，或把涉及的内容说得更具体。".into());
            }

            if turn.stop_reason != "tool_use" {
                return Ok(());
            }

            let mut results = Vec::new();
            let mut layout_changed = false;

            for call in &turn.tool_uses {
                let deps = ToolDeps {
                    backend,
                    layout,
                    catalog,
                };
                let outcome = tools::dispatch(&call.name, &call.input, &deps);
                layout_changed |= outcome.layout_changed;

                let _ = app.emit(
                    EVENT_TOOL,
                    json!({
                        "name": call.name,
                        "ok": !outcome.is_error,
                        "summary": call.input.get("summary"),
                    }),
                );

                // 失败的工具也要回一条 tool_result（带 is_error），不能不回：
                // 少一条 tool_result，下一轮请求整个会被拒。
                results.push(json!({
                    "type": "tool_result",
                    "tool_use_id": call.id,
                    "content": outcome.content,
                    "is_error": outcome.is_error,
                }));
            }

            // 一轮里所有 tool_result 放同一条 user 消息，不拆开。
            self.history.push(Message::tool_results(results));

            if layout_changed {
                let _ = app.emit(EVENT_LAYOUT_CHANGED, json!({}));
            }

            if round + 1 == MAX_TOOL_ROUNDS {
                return Err(format!(
                    "工具调用超过 {MAX_TOOL_ROUNDS} 轮仍未收敛，已停下。把问题拆小一点再试。"
                ));
            }
        }

        Ok(())
    }
}
