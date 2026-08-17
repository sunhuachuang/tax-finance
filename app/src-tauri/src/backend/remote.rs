//! 瘦客户端：打到常驻 host 上的 `taxweb`。移动端的默认路径。
//!
//! 账本只有一份，永远在 host 上——手机不存副本，也就没有同步冲突，
//! provenance 链不会分叉。
//!
//! HTTP 客户端是手写的，和 `taxweb::http` 的服务端一样：只需要 GET/POST
//! 两个动词和 JSON 响应，为此拉一个 HTTP 栈进来不划算。这条链路假定跑在
//! Tailscale / WireGuard 隧道里（隧道负责加密和设备认证），**绝不能暴露到公网**。

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;

use super::{Backend, BackendResult, mime_for};

const TIMEOUT: Duration = Duration::from_secs(15);
/// 上传一份扫描件可能要几十秒，尤其在移动网络上。
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(120);

/// 文件名进 query string，所以空格、`&`、中文都要转义，否则服务端会把
/// 文件名截断成另一个参数。只放行 RFC 3986 的 unreserved 字符。
fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

pub struct RemoteBackend {
    base_url: String,
}

impl RemoteBackend {
    pub fn new(base_url: impl Into<String>) -> Self {
        RemoteBackend {
            base_url: base_url.into(),
        }
    }

    fn request(&self, method: &str, path: &str) -> BackendResult {
        self.send(method, path, None, &[])
    }

    fn send(
        &self,
        method: &str,
        path: &str,
        content_type: Option<&str>,
        body: &[u8],
    ) -> BackendResult {
        if self.base_url.is_empty() {
            return Err("未配置 host：设置环境变量 FINANCE_HOST（如 http://mac-mini:5710）".into());
        }
        let (host, port) = split_authority(&self.base_url)?;

        let mut stream = TcpStream::connect((host.as_str(), port))
            .map_err(|e| format!("连接 {host}:{port} 失败：{e}"))?;
        stream.set_read_timeout(Some(TIMEOUT)).ok();
        // 上传一份几 MB 的扫描件比一次查询慢，写超时单独放宽。
        stream.set_write_timeout(Some(UPLOAD_TIMEOUT)).ok();

        let content_type = content_type
            .map(|t| format!("Content-Type: {t}\r\n"))
            .unwrap_or_default();
        write!(
            stream,
            "{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\n\
             Accept: application/json\r\n{content_type}\
             Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .map_err(|e| format!("发送请求失败：{e}"))?;
        if !body.is_empty() {
            stream
                .write_all(body)
                .map_err(|e| format!("发送文件内容失败：{e}"))?;
        }
        stream.flush().ok();

        // 服务端回 `Connection: close`，读到 EOF 即为完整响应。
        let mut raw = Vec::new();
        stream
            .read_to_end(&mut raw)
            .map_err(|e| format!("读取响应失败：{e}"))?;

        let text = String::from_utf8_lossy(&raw);
        let (head, body) = text
            .split_once("\r\n\r\n")
            .ok_or("响应不是合法的 HTTP：找不到 header 结束标记")?;
        let status = head
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse::<u16>().ok())
            .ok_or("响应不是合法的 HTTP：读不到状态码")?;

        let value: Value = serde_json::from_str(body.trim())
            .map_err(|e| format!("响应不是合法 JSON（HTTP {status}）：{e}"))?;

        if status == 200 {
            Ok(value)
        } else {
            // taxweb 的错误体形如 {"error": "..."}。
            let message = value
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("未知错误");
            Err(format!("host 返回 HTTP {status}：{message}"))
        }
    }
}

impl Backend for RemoteBackend {
    fn overview(&mut self) -> BackendResult {
        self.request("GET", "/api/overview")
    }

    fn gst(&mut self, date: Option<String>, frequency: Option<String>) -> BackendResult {
        let mut query = Vec::new();
        if let Some(d) = date.filter(|s| !s.is_empty()) {
            query.push(format!("date={d}"));
        }
        if let Some(f) = frequency.filter(|s| !s.is_empty()) {
            query.push(format!("frequency={f}"));
        }
        let path = if query.is_empty() {
            "/api/gst".to_string()
        } else {
            format!("/api/gst?{}", query.join("&"))
        };
        self.request("GET", &path)
    }

    fn ir3(&mut self, year: String) -> BackendResult {
        self.request("GET", &format!("/api/ir3?year={year}"))
    }

    /// 文件原样作为请求体发给 host，不做 multipart——一次一个文件不需要解析器。
    /// 文档落在 host 的账本里，手机上不留副本。
    fn ingest_document(&mut self, path: String) -> BackendResult {
        let path = PathBuf::from(path);
        let bytes =
            std::fs::read(&path).map_err(|e| format!("读取 {} 失败：{e}", path.display()))?;
        if bytes.is_empty() {
            return Err(format!("{} 是空文件", path.display()));
        }

        let filename = path
            .file_name()
            .map(|name| percent_encode(&name.to_string_lossy()))
            .unwrap_or_default();

        self.send(
            "POST",
            &format!("/api/documents?filename={filename}"),
            Some(&mime_for(&path)),
            &bytes,
        )
    }

    fn document(&mut self, document_id: String) -> BackendResult {
        let mut value = self.request("GET", &format!("/api/documents/{document_id}"))?;
        // host 回的是文件在 **它** 那台机器上的路径。对这台客户端来说那条路径
        // 不存在，留着只会让界面提供一个打不开的按钮。
        if let Some(object) = value.as_object_mut() {
            object.insert("local_path".into(), Value::Null);
        }
        Ok(value)
    }

    fn set_document_status(&mut self, document_id: String, to: String) -> BackendResult {
        self.request(
            "POST",
            &format!("/api/documents/{document_id}/status?to={to}"),
        )
    }

    fn approve_entry(&mut self, entry_id: String) -> BackendResult {
        self.request("POST", &format!("/api/entries/{entry_id}/approve"))
    }

    fn reject_entry(&mut self, entry_id: String) -> BackendResult {
        self.request("POST", &format!("/api/entries/{entry_id}/reject"))
    }
}

/// `http://host:5710` → `("host", 5710)`。只认 http——https 由隧道之外的方案负责，
/// 手写一个 TLS 栈不在讨论范围内。
fn split_authority(base_url: &str) -> Result<(String, u16), String> {
    let rest = base_url
        .strip_prefix("http://")
        .ok_or_else(|| format!("FINANCE_HOST 必须以 http:// 开头，收到 {base_url}"))?;
    let authority = rest.split('/').next().unwrap_or(rest);
    match authority.rsplit_once(':') {
        Some((host, port)) => {
            let port = port
                .parse()
                .map_err(|_| format!("{port} 不是合法端口号"))?;
            Ok((host.to_string(), port))
        }
        None => Ok((authority.to_string(), 80)),
    }
}

#[cfg(test)]
mod tests {
    use super::{RemoteBackend, split_authority};
    use crate::backend::Backend;

    /// 需要一台活的 taxweb，所以默认不跑：
    ///
    /// ```sh
    /// cargo run -p taxweb -- --demo          # 另开一个终端
    /// FINANCE_HOST=http://127.0.0.1:5710 cargo test -- --ignored
    /// ```
    #[test]
    #[ignore = "需要一台在跑的 taxweb，用 FINANCE_HOST 指定"]
    fn talks_to_a_live_taxweb() {
        let host = std::env::var("FINANCE_HOST").expect("请设置 FINANCE_HOST");
        let mut backend = RemoteBackend::new(host);

        let overview = backend.overview().expect("overview 应当成功");
        assert!(
            overview.get("review_drafts").is_some_and(|v| v.is_array()),
            "overview 里应当有 review_drafts 数组，实际拿到 {overview}"
        );

        // 错误路径：缺 year 参数时 taxweb 回 400，客户端要把它变成 Err 而不是当成数据。
        let err = backend.ir3(String::new()).unwrap_err();
        assert!(err.contains("400"), "期望 HTTP 400，实际 {err}");
    }

    #[test]
    #[ignore = "需要一台在跑的 taxweb，用 FINANCE_HOST 指定"]
    fn uploads_a_document_to_the_host() {
        let host = std::env::var("FINANCE_HOST").expect("请设置 FINANCE_HOST");
        let mut backend = RemoteBackend::new(host);

        // 文件名带空格和中文，走一遍百分号编码。内容带上时间戳，
        // 免得重跑测试时撞上内容寻址去重。
        let dir = std::env::temp_dir();
        let file = dir.join(format!("远程 upload {}.pdf", std::process::id()));
        let bytes = format!("fake pdf {:?}", std::time::SystemTime::now());
        std::fs::write(&file, &bytes).unwrap();

        let first = backend
            .ingest_document(file.to_string_lossy().into_owned())
            .expect("上传应当成功");
        assert_eq!(first["duplicate"], false);
        assert_eq!(first["document"]["status"], "pending_extraction");
        assert_eq!(first["document"]["mime"], "application/pdf");
        assert_eq!(
            first["document"]["original_filename"],
            format!("远程 upload {}.pdf", std::process::id()),
            "文件名必须原样到达 host，空格和中文都不能被 query string 吃掉"
        );

        // 同样的字节再传一次 —— 身份是内容哈希，host 认得出来。
        let second = backend
            .ingest_document(file.to_string_lossy().into_owned())
            .unwrap();
        assert_eq!(second["duplicate"], true);
        assert_eq!(second["document"]["id"], first["document"]["id"]);

        // 取详情：host 会带上它自己那台机器的路径，客户端必须把它抹掉，
        // 否则界面会提供一个打不开的「用系统程序打开」。
        let id = first["document"]["id"].as_str().unwrap().to_string();
        let detail = backend.document(id.clone()).unwrap();
        assert!(detail["local_path"].is_null(), "远程模式不该给出本地路径");
        assert_eq!(detail["document"]["status"], "pending_extraction");

        // 人的两个决定也要能远程做。
        backend
            .set_document_status(id.clone(), "ignored".into())
            .unwrap();
        assert_eq!(
            backend.document(id.clone()).unwrap()["document"]["status"],
            "ignored"
        );
        let err = backend
            .set_document_status(id, "extracted".into())
            .unwrap_err();
        assert!(err.contains("400"), "期望 host 拒绝，实际 {err}");

        std::fs::remove_file(&file).ok();
    }

    #[test]
    fn parses_host_and_port() {
        assert_eq!(
            split_authority("http://mac-mini:5710").unwrap(),
            ("mac-mini".to_string(), 5710)
        );
        assert_eq!(
            split_authority("http://127.0.0.1:5710/").unwrap(),
            ("127.0.0.1".to_string(), 5710)
        );
        assert_eq!(
            split_authority("http://example").unwrap(),
            ("example".to_string(), 80)
        );
        assert!(split_authority("https://example:443").is_err());
    }
}
