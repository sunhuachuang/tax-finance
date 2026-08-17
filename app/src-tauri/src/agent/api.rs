//! Claude Messages API 的最小客户端。
//!
//! Rust 没有官方 Anthropic SDK，所以直接打 HTTP。只实现这个 app 用得到的部分：
//! 流式、工具调用、adaptive thinking。
//!
//! **API key 只存在于这个进程里，绝不进 webview。** 前端拿不到它，
//! 页面里任何东西（包括 agent 生成的布局）都偷不走。

use std::collections::BTreeMap;

use futures_util::StreamExt as _;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const API_VERSION: &str = "2023-06-01";

/// Claude Opus 5。默认模型，除非用户明确要求换。
pub const MODEL: &str = "claude-opus-5";

/// 上限，不是目标。Claude Opus 5 默认开思考，而 max_tokens 是
/// **思考 + 回复正文** 的总闸门——给小了会在半句话中间被截断。
///
/// 但也不能一味往大给：**声明的 max_tokens 会按上限计入每分钟输出配额**，
/// 哪怕实际只输出几百 token。之前设成 64000 时，配额紧的凭证一个请求就顶满，
/// 直接 429。16000 对「回答一个问题 + 吐一份布局 JSON」绰绰有余
/// （四页布局的完整文档大约 3–4K token）。
const MAX_TOKENS: u32 = 16_000;

/// 429 / 529 的重试次数。原生 HTTP 没有 SDK 那套自动退避，得自己来。
const MAX_RETRIES: u32 = 3;

/// 服务端没给 `retry-after` 时的退避基数。
const BACKOFF_BASE_SECS: u64 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl Default for Effort {
    /// 交互式聊天里延迟是体感的一部分，而 Opus 5 在 medium 上已经很强。
    /// 觉得布局改得不够聪明就往上调——这是主要的质量/延迟旋钮。
    fn default() -> Self {
        Effort::Medium
    }
}

/// 对话里的一条消息。`content` 保持成 JSON 数组原样回传——
/// thinking 块必须原封不动带回去，任何改写都会让下一轮 400。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: Value,
}

impl Message {
    pub fn user_text(text: impl Into<String>) -> Self {
        Message {
            role: "user".into(),
            content: json!([{ "type": "text", "text": text.into() }]),
        }
    }

    pub fn assistant(content: Value) -> Self {
        Message {
            role: "assistant".into(),
            content,
        }
    }

    /// 一轮里所有 tool_result 必须放在**同一条** user 消息里——
    /// 拆开发会让模型逐渐不再并行调用工具。
    pub fn tool_results(results: Vec<Value>) -> Self {
        Message {
            role: "user".into(),
            content: Value::Array(results),
        }
    }
}

/// 流式过程中吐给 UI 的增量。
#[derive(Clone, Debug)]
pub enum Delta {
    /// 回复正文的一段。
    Text(String),
    /// 思考摘要的一段。原始思维链永远拿不到，这是模型自己的总结。
    Thinking(String),
    /// 开始调用某个工具（此时参数还没流完）。
    ToolStarted { name: String },
    /// 给用户的一句进度说明（如正在退避重试）。不进对话历史。
    Notice(String),
}

/// 一轮对话的结果。
#[derive(Debug)]
pub struct Turn {
    /// 助手这一轮的完整 content，原样用于回传。
    pub content: Value,
    pub stop_reason: String,
    /// 需要执行的工具调用。
    pub tool_uses: Vec<ToolUse>,
}

#[derive(Clone, Debug)]
pub struct ToolUse {
    pub id: String,
    pub name: String,
    pub input: Value,
}

/// 凭证的两种形态。**它们走不同的 header**，混用会直接 401——
/// 把 OAuth token 塞进 `x-api-key` 是这个错误最常见的来源。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Credential {
    /// Console 里建的 API key（`sk-ant-api03-…`）。长期有效。
    ApiKey(String),
    /// OAuth access token（`sk-ant-oat01-…`），来自 `ant auth login`
    /// 之类的登录流程。**短期有效，会过期**。
    OAuth(String),
    /// claude.ai 的网页会话 key（`sk-ant-sid01-…`）。它不是 API 凭证，
    /// 认出来是为了给一句有用的报错，而不是让用户对着 401 猜。
    SessionKey(String),
    /// 第三方 Anthropic 兼容端点（DeepSeek / Kimi / GLM / MiniMax…）的 key。
    /// 这些端点按 Claude Code 的约定收 `Authorization: Bearer`，
    /// 而且不认 Anthropic 专有的 beta header。
    ThirdParty(String),
}

impl Credential {
    pub fn parse(raw: &str) -> Self {
        let raw = raw.trim().to_string();
        if raw.starts_with("sk-ant-oat") {
            Credential::OAuth(raw)
        } else if raw.starts_with("sk-ant-sid") {
            Credential::SessionKey(raw)
        } else {
            Credential::ApiKey(raw)
        }
    }

    fn apply(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self {
            // 两个 header 只能有一个：同时发 x-api-key 和 Authorization 会被拒。
            Credential::ApiKey(key) | Credential::SessionKey(key) => {
                request.header("x-api-key", key)
            }
            Credential::OAuth(token) => request
                .header("authorization", format!("Bearer {token}"))
                .header("anthropic-beta", "oauth-2025-04-20"),
            // 不带 anthropic-beta：那是 Anthropic 专有的，第三方端点不认。
            Credential::ThirdParty(key) => {
                request.header("authorization", format!("Bearer {key}"))
            }
        }
    }

    /// 401 时补一句针对这种凭证的解释。干看 "API key is invalid" 没法判断
    /// 是打错了、过期了、还是压根用错了类型。
    fn unauthorized_hint(&self) -> &'static str {
        match self {
            Credential::ApiKey(_) => {
                "这看起来是一把 API key。确认它是从 console.anthropic.com 的 API Keys 页面\
                 复制的完整字符串（通常以 sk-ant-api 开头），没有多余空格，也没有被撤销。"
            }
            Credential::OAuth(_) => {
                "这是一个 OAuth token（sk-ant-oat…），已按 Bearer 方式发送。\
                 这类 token 是短期的，过期后必须重新获取。要长期使用请改用 \
                 console.anthropic.com 上创建的 API key。"
            }
            Credential::SessionKey(_) => {
                "这是 claude.ai 的网页会话 key（sk-ant-sid…），它不能用于 API。\
                 请到 console.anthropic.com 的 API Keys 页面创建一把 API key。"
            }
            Credential::ThirdParty(_) => {
                "这是发给第三方端点的 key。确认它对应的是你填的那个服务地址，\
                 并且该服务的 Anthropic 兼容端点已开通。"
            }
        }
    }

    /// 429 时补一句。OAuth token 走的是订阅额度，限流阈值和计费 API key 不是一回事。
    fn rate_limit_hint(&self) -> &'static str {
        match self {
            Credential::OAuth(_) | Credential::SessionKey(_) => {
                "你用的是订阅登录态（sk-ant-oat…），它的额度按 Claude 应用的用法配的，\
                 不适合程序化调用，很容易撞限流。换一把 console.anthropic.com 上创建的 \
                 API key 会稳定得多。也可以过一会儿再试，或把问题拆小。"
            }
            Credential::ApiKey(_) => {
                "稍等片刻再试。持续限流说明这个账号的每分钟额度不够——\
                 可以在 console.anthropic.com 查看当前用量和限额。"
            }
            Credential::ThirdParty(_) => {
                "稍等片刻再试，或到该服务的控制台查看用量与限额。"
            }
        }
    }
}

/// 服务端建议的等待秒数。给了就听它的，别自作聪明。
fn retry_after(response: &reqwest::Response) -> Option<u64> {
    response
        .headers()
        .get("retry-after")?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// 限流相关的 header。真正的配额限流会带上这些；**一个都没有**反而是重要信号——
/// 说明这多半不是「用超了」，而是这类凭证在这个端点上根本没有配额。
fn rate_limit_headers(response: &reqwest::Response) -> Vec<String> {
    response
        .headers()
        .iter()
        .filter(|(name, _)| {
            let name = name.as_str();
            name.starts_with("anthropic-ratelimit") || name == "retry-after"
        })
        .filter_map(|(name, value)| {
            value.to_str().ok().map(|v| format!("{name}: {v}"))
        })
        .collect()
}

pub struct Client {
    http: reqwest::Client,
    credential: Credential,
    endpoint: String,
    model: Option<String>,
    /// 是不是 Anthropic 官方端点。第三方兼容端点（DeepSeek / Kimi / GLM…）
    /// 只实现了 Messages API 的公共部分，`thinking` 和 `output_config.effort`
    /// 是 Anthropic 专有的，发过去可能 400。
    native: bool,
}

/// 模型名。官方端点可以缺省，第三方**必须**填——
/// 把 `claude-opus-5` 发给 DeepSeek 只会换来一句看不懂的报错。
fn resolve_model<'a>(model: &'a Option<String>, native: bool) -> Result<&'a str, String> {
    match model.as_deref() {
        Some(m) if !m.is_empty() => Ok(m),
        _ if native => Ok(MODEL),
        _ => Err("填了模型服务地址，但没填模型名。第三方服务不认识 Anthropic 的模型名，\
                  必须填它自己的（比如 DeepSeek 是 deepseek-v4-flash）。"
            .to_string()),
    }
}

impl Client {
    /// `base_url` 留空即 Anthropic 官方；填了就打第三方的 Anthropic 兼容端点
    /// （DeepSeek / Kimi / GLM / MiniMax 都提供，当初为兼容 Claude Code 做的）。
    /// 协议完全一样，所以流式、工具调用这些代码一行都不用改。
    pub fn new(api_key: String, base_url: Option<String>, model: Option<String>) -> Self {
        let base = base_url
            .map(|b| b.trim().trim_end_matches('/').to_string())
            .filter(|b| !b.is_empty());
        let native = base.is_none();
        let base = base.unwrap_or_else(|| DEFAULT_BASE_URL.to_string());

        Client {
            http: reqwest::Client::new(),
            // 第三方端点按 Claude Code 的约定收 Bearer，不看 key 前缀。
            credential: if native {
                Credential::parse(&api_key)
            } else {
                Credential::ThirdParty(api_key.trim().to_string())
            },
            endpoint: format!("{base}/v1/messages"),
            model: model.map(|m| m.trim().to_string()).filter(|m| !m.is_empty()),
            native,
        }
    }

    /// 发一轮请求，边流边回调，最后返回聚合好的这一轮。
    pub async fn stream_turn(
        &self,
        system: &str,
        tools: &Value,
        messages: &[Message],
        effort: Effort,
        mut on_delta: impl FnMut(Delta),
    ) -> Result<Turn, String> {
        let body = request_body(resolve_model(&self.model, self.native)?, self.native, system, tools, messages, effort);

        // 限流和过载是暂时的，自己退避重试——官方 SDK 默认就这么做，
        // 原生 HTTP 没有这层，不补的话用户看到的就是一条冷冰冰的 429。
        let mut attempt = 0;
        let response = loop {
            let request = self
                .http
                .post(&self.endpoint)
                .header("anthropic-version", API_VERSION)
                .header("content-type", "application/json");

            let response = self
                .credential
                .apply(request)
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("请求 Claude 失败：{e}"))?;

            let status = response.status();
            if status.is_success() {
                break response;
            }

            let retryable = status == reqwest::StatusCode::TOO_MANY_REQUESTS
                || status.as_u16() == 529
                || status.is_server_error();

            if retryable && attempt < MAX_RETRIES {
                let wait = retry_after(&response)
                    .unwrap_or(BACKOFF_BASE_SECS * 2_u64.pow(attempt));
                attempt += 1;
                on_delta(Delta::Notice(format!(
                    "被限流，{wait} 秒后第 {attempt} 次重试…"
                )));
                tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
                continue;
            }

            let limits = rate_limit_headers(&response);
            let detail = response.text().await.unwrap_or_default();
            return Err(match status {
                // 光看 "API key is invalid" 分不清是打错了、过期了、还是用错了
                // 凭证类型，而这三种的处理方式完全不同。
                reqwest::StatusCode::UNAUTHORIZED => {
                    format!("凭证被拒（401）。{}", self.credential.unauthorized_hint())
                }
                reqwest::StatusCode::TOO_MANY_REQUESTS => {
                    // 带回限额 header：有值说明确实是用超了，等一等能好；
                    // 一个都没有说明这个凭证在这个端点上压根没有配额，
                    // 再等也不会通——这两种情况的处理方式完全相反。
                    let diagnosis = if limits.is_empty() {
                        "服务端没有返回任何限额信息（anthropic-ratelimit-*），\
                         这通常意味着不是「用超了」，而是这类凭证没有这个端点的配额——\
                         再等再重试都不会通。"
                            .to_string()
                    } else {
                        format!("当前限额：{}", limits.join("；"))
                    };
                    format!(
                        "已限流（429），重试 {MAX_RETRIES} 次仍未通过。{diagnosis}\n\n{}",
                        self.credential.rate_limit_hint()
                    )
                }
                other => format!("Claude 返回 {other}：{detail}"),
            });
        };

        let mut acc = Accumulator::default();
        let mut buffer = String::new();
        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("读取流失败：{e}"))?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            // SSE 事件以空行分隔；只认 data: 行，事件类型在 JSON 的 type 字段里。
            while let Some(split) = buffer.find("\n\n") {
                let raw = buffer[..split].to_string();
                buffer.drain(..split + 2);
                for line in raw.lines() {
                    let Some(payload) = line.strip_prefix("data:") else {
                        continue;
                    };
                    let payload = payload.trim();
                    if payload.is_empty() {
                        continue;
                    }
                    let event: Value = serde_json::from_str(payload)
                        .map_err(|e| format!("流里出现非法 JSON：{e}"))?;
                    acc.feed(&event, &mut on_delta);
                }
            }
        }

        acc.finish()
    }
}

/// 请求体。抽出来是为了能在没有 API key 的情况下验证参数形状——
/// 这些字段错一个，行为就完全不同（比如 display 默认是 omitted，
/// 漏掉它界面上只会看到一段长时间的空白）。
fn request_body(
    model: &str,
    native: bool,
    system: &str,
    tools: &Value,
    messages: &[Message],
    effort: Effort,
) -> Value {
    let mut body = json!({
        "model": model,
        "max_tokens": MAX_TOKENS,
        // system 和 tools 每一轮都一样，缓存住它们；
        // 多轮对话里只有新消息按全价计费。断点放在 system 最后一块上，
        // 渲染顺序是 tools → system → messages，所以这一个断点把两者都盖住。
        "system": [{
            "type": "text",
            "text": system,
            "cache_control": { "type": "ephemeral" },
        }],
        "tools": tools,
        "stream": true,
        "messages": messages,
    });

    // thinking 和 effort 是 Anthropic 模型专有的。第三方兼容端点只实现了
    // Messages API 的公共部分，把这两个发过去有的会直接 400。
    if native && let Some(object) = body.as_object_mut() {
        // 思考在 Opus 5 上默认就开；display 默认 omitted，
        // 那样界面上只会看到一段长时间的空白，所以显式要摘要。
        object.insert(
            "thinking".into(),
            json!({ "type": "adaptive", "display": "summarized" }),
        );
        object.insert("output_config".into(), json!({ "effort": effort }));
    }

    body
}

/// 把 SSE 增量攒回一条完整的助手消息。
#[derive(Default)]
struct Accumulator {
    /// index -> 正在拼装的块。
    blocks: BTreeMap<usize, PartialBlock>,
    stop_reason: Option<String>,
    error: Option<String>,
}

enum PartialBlock {
    Text(String),
    /// 思考块要连同 signature 一起原样回传，所以完整保存原始块。
    Thinking { raw: Value, text: String },
    ToolUse {
        id: String,
        name: String,
        json: String,
    },
    /// 认不出的块型（未来新增的），原样留着回传，不解读。
    Opaque(Value),
}

impl Accumulator {
    fn feed(&mut self, event: &Value, on_delta: &mut impl FnMut(Delta)) {
        match event.get("type").and_then(Value::as_str) {
            Some("content_block_start") => {
                let Some(index) = event.get("index").and_then(Value::as_u64) else {
                    return;
                };
                let block = event.get("content_block").cloned().unwrap_or(Value::Null);
                let partial = match block.get("type").and_then(Value::as_str) {
                    Some("text") => PartialBlock::Text(String::new()),
                    Some("thinking") => PartialBlock::Thinking {
                        raw: block.clone(),
                        text: String::new(),
                    },
                    Some("tool_use") => {
                        let name = block
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        on_delta(Delta::ToolStarted { name: name.clone() });
                        PartialBlock::ToolUse {
                            id: block
                                .get("id")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                            name,
                            json: String::new(),
                        }
                    }
                    _ => PartialBlock::Opaque(block),
                };
                self.blocks.insert(index as usize, partial);
            }

            Some("content_block_delta") => {
                let Some(index) = event.get("index").and_then(Value::as_u64) else {
                    return;
                };
                let Some(slot) = self.blocks.get_mut(&(index as usize)) else {
                    return;
                };
                let delta = event.get("delta").cloned().unwrap_or(Value::Null);
                match (delta.get("type").and_then(Value::as_str), slot) {
                    (Some("text_delta"), PartialBlock::Text(buf)) => {
                        let piece = delta.get("text").and_then(Value::as_str).unwrap_or("");
                        buf.push_str(piece);
                        on_delta(Delta::Text(piece.to_string()));
                    }
                    (Some("thinking_delta"), PartialBlock::Thinking { text, .. }) => {
                        let piece = delta.get("thinking").and_then(Value::as_str).unwrap_or("");
                        text.push_str(piece);
                        on_delta(Delta::Thinking(piece.to_string()));
                    }
                    // 工具参数是逐段 JSON 文本流过来的，攒齐才能解析。
                    (Some("input_json_delta"), PartialBlock::ToolUse { json, .. }) => {
                        json.push_str(delta.get("partial_json").and_then(Value::as_str).unwrap_or(""));
                    }
                    // signature_delta 等：原样跳过，聚合时用原始块。
                    _ => {}
                }
            }

            Some("message_delta") => {
                if let Some(reason) = event.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    self.stop_reason = Some(reason.to_string());
                }
            }

            Some("error") => {
                self.error = Some(
                    event
                        .pointer("/error/message")
                        .and_then(Value::as_str)
                        .unwrap_or("未知错误")
                        .to_string(),
                );
            }

            _ => {}
        }
    }

    fn finish(self) -> Result<Turn, String> {
        if let Some(message) = self.error {
            return Err(format!("Claude 报错：{message}"));
        }

        let mut content = Vec::new();
        let mut tool_uses = Vec::new();

        for (_, block) in self.blocks {
            match block {
                PartialBlock::Text(text) => {
                    content.push(json!({ "type": "text", "text": text }));
                }
                PartialBlock::Thinking { mut raw, text } => {
                    // signature 等字段留在 raw 里，只把流回来的正文填进去。
                    if let Some(object) = raw.as_object_mut() {
                        object.insert("thinking".into(), Value::String(text));
                    }
                    content.push(raw);
                }
                PartialBlock::ToolUse { id, name, json } => {
                    let input: Value = if json.trim().is_empty() {
                        json!({})
                    } else {
                        serde_json::from_str(&json)
                            .map_err(|e| format!("工具 {name} 的参数不是合法 JSON：{e}"))?
                    };
                    content.push(json!({
                        "type": "tool_use", "id": id, "name": name, "input": input,
                    }));
                    tool_uses.push(ToolUse { id, name, input });
                }
                PartialBlock::Opaque(raw) => content.push(raw),
            }
        }

        Ok(Turn {
            content: Value::Array(content),
            stop_reason: self.stop_reason.unwrap_or_else(|| "end_turn".into()),
            tool_uses,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_are_classified_by_prefix() {
        assert_eq!(
            Credential::parse("sk-ant-api03-abc"),
            Credential::ApiKey("sk-ant-api03-abc".into())
        );
        assert_eq!(
            Credential::parse("sk-ant-oat01-abc"),
            Credential::OAuth("sk-ant-oat01-abc".into())
        );
        assert_eq!(
            Credential::parse("sk-ant-sid01-abc"),
            Credential::SessionKey("sk-ant-sid01-abc".into())
        );
        // 认不出的当 API key 处理——比拒绝服务好，让服务端去判断。
        assert!(matches!(Credential::parse("something-else"), Credential::ApiKey(_)));
        // 粘贴时常带的首尾空白要吃掉，否则会变成一个 401 的谜题。
        assert_eq!(
            Credential::parse("  sk-ant-oat01-abc\n"),
            Credential::OAuth("sk-ant-oat01-abc".into())
        );
    }

    #[test]
    fn each_credential_kind_gets_its_own_401_explanation() {
        // 三种凭证的 401 处理方式完全不同，所以提示必须能区分。
        let api = Credential::parse("sk-ant-api03-x").unauthorized_hint();
        let oauth = Credential::parse("sk-ant-oat01-x").unauthorized_hint();
        let session = Credential::parse("sk-ant-sid01-x").unauthorized_hint();
        assert_ne!(api, oauth);
        assert_ne!(oauth, session);
        assert!(oauth.contains("过期"), "OAuth 提示要说清它会过期：{oauth}");
        assert!(session.contains("不能用于 API"), "会话 key 提示要说清它不是 API 凭证：{session}");
    }

    #[test]
    fn the_request_body_carries_the_parameters_that_change_behaviour() {
        let body = request_body(MODEL, true, "你是助手", &json!([]), &[Message::user_text("hi")], Effort::Medium);

        assert_eq!(body["model"], MODEL);
        // 思考默认就开，但 display 默认 omitted —— 不显式要摘要的话
        // 界面上只会看到一段长时间的空白。
        assert_eq!(body["thinking"]["type"], "adaptive");
        assert_eq!(body["thinking"]["display"], "summarized");
        // effort 是小写字符串，不是对象。
        assert_eq!(body["output_config"]["effort"], "medium");
        assert_eq!(body["stream"], true);
        // max_tokens 两头都有约束，所以断言是个区间：
        //   下限——它是思考 + 正文的总闸门，给小了会截断在半句话；
        //   上限——声明值按上限计入每分钟输出配额，给大了直接 429。
        let max_tokens = body["max_tokens"].as_u64().unwrap();
        assert!(max_tokens >= 8_000, "太小会把回复截断在半句话：{max_tokens}");
        assert!(max_tokens <= 32_000, "太大会顶满每分钟输出配额，换来 429：{max_tokens}");
    }

    #[test]
    fn a_third_party_endpoint_gets_no_anthropic_only_fields() {
        // DeepSeek / Kimi / GLM 的兼容端点只实现了 Messages API 的公共部分，
        // thinking 和 output_config 发过去可能直接 400。
        let body = request_body(
            "deepseek-v4-flash",
            false,
            "prompt",
            &json!([]),
            &[],
            Effort::Medium,
        );
        assert_eq!(body["model"], "deepseek-v4-flash");
        assert!(body.get("thinking").is_none(), "第三方端点不该收到 thinking");
        assert!(
            body.get("output_config").is_none(),
            "第三方端点不该收到 output_config"
        );
        // 公共部分照常发。
        assert_eq!(body["stream"], true);
        assert!(body.get("tools").is_some());
    }

    #[test]
    fn a_third_party_key_goes_on_bearer_regardless_of_prefix() {
        // 第三方 key 不遵守 sk-ant- 的命名，所以不能按前缀判断；
        // 按 Claude Code 的约定，兼容端点一律收 Authorization: Bearer。
        let client = Client::new(
            "sk-anything".into(),
            Some("https://api.deepseek.com/anthropic".into()),
            Some("deepseek-v4-flash".into()),
        );
        assert!(matches!(client.credential, Credential::ThirdParty(_)));
        assert_eq!(client.endpoint, "https://api.deepseek.com/anthropic/v1/messages");
        assert!(!client.native);
    }

    #[test]
    fn a_third_party_endpoint_without_a_model_name_fails_loudly() {
        // 静默回落到 claude-opus-5 会把这个配置错误变成一句第三方服务的
        // 「未知模型」报错，看不出根因在哪。
        let client = Client::new(
            "sk-x".into(),
            Some("https://api.deepseek.com/anthropic".into()),
            None,
        );
        let err = resolve_model(&client.model, client.native).unwrap_err();
        assert!(err.contains("没填模型名"), "{err}");
    }

    #[test]
    fn an_empty_base_url_falls_back_to_anthropic() {
        // 设置界面清空输入框会传空串，不能因此拼出一个 "/v1/messages" 的坏地址。
        let client = Client::new("sk-ant-api03-x".into(), Some("   ".into()), Some(String::new()));
        assert_eq!(client.endpoint, "https://api.anthropic.com/v1/messages");
        assert_eq!(resolve_model(&client.model, client.native).unwrap(), MODEL);
        assert!(client.native);
    }

    #[test]
    fn the_cache_breakpoint_sits_on_the_system_block() {
        let body = request_body(MODEL, true, "prompt", &json!([]), &[], Effort::default());
        // 渲染顺序是 tools → system → messages，所以 system 上这一个断点
        // 把工具定义也一起缓存了。
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
    }

    fn drive(events: &[Value]) -> Turn {
        let mut acc = Accumulator::default();
        let mut seen = Vec::new();
        for event in events {
            acc.feed(event, &mut |d| seen.push(d));
        }
        acc.finish().unwrap()
    }

    #[test]
    fn text_deltas_accumulate_into_one_block() {
        let turn = drive(&[
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"把 GST "}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"挪到顶部"}}),
            json!({"type":"message_delta","delta":{"stop_reason":"end_turn"}}),
        ]);
        assert_eq!(turn.content[0]["text"], "把 GST 挪到顶部");
        assert_eq!(turn.stop_reason, "end_turn");
    }

    #[test]
    fn tool_input_json_is_reassembled_from_fragments() {
        let turn = drive(&[
            json!({"type":"content_block_start","index":0,
                   "content_block":{"type":"tool_use","id":"toolu_1","name":"apply_layout"}}),
            json!({"type":"content_block_delta","index":0,
                   "delta":{"type":"input_json_delta","partial_json":"{\"doc\":"}}),
            json!({"type":"content_block_delta","index":0,
                   "delta":{"type":"input_json_delta","partial_json":"{\"pages\":[]}}"}}),
            json!({"type":"message_delta","delta":{"stop_reason":"tool_use"}}),
        ]);
        assert_eq!(turn.stop_reason, "tool_use");
        assert_eq!(turn.tool_uses.len(), 1);
        assert_eq!(turn.tool_uses[0].name, "apply_layout");
        assert_eq!(turn.tool_uses[0].input["doc"]["pages"], json!([]));
    }

    #[test]
    fn thinking_blocks_keep_their_signature_for_replay() {
        let turn = drive(&[
            json!({"type":"content_block_start","index":0,
                   "content_block":{"type":"thinking","thinking":"","signature":"sig-abc"}}),
            json!({"type":"content_block_delta","index":0,
                   "delta":{"type":"thinking_delta","thinking":"先看看当前布局"}}),
        ]);
        // signature 必须原样保留——改写过的 thinking 块下一轮会被 API 拒掉。
        assert_eq!(turn.content[0]["signature"], "sig-abc");
        assert_eq!(turn.content[0]["thinking"], "先看看当前布局");
    }

    #[test]
    fn an_error_event_becomes_an_error_not_a_silent_empty_turn() {
        let mut acc = Accumulator::default();
        acc.feed(
            &json!({"type":"error","error":{"type":"overloaded_error","message":"overloaded"}}),
            &mut |_| {},
        );
        assert!(acc.finish().unwrap_err().contains("overloaded"));
    }

    #[test]
    fn unknown_block_types_are_carried_through_untouched() {
        let turn = drive(&[json!({
            "type":"content_block_start","index":0,
            "content_block":{"type":"future_block","payload":42}
        })]);
        assert_eq!(turn.content[0]["payload"], 42);
    }
}
