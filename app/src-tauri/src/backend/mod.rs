//! 数据来源的抽象接缝。
//!
//! UI 侧永远只调 `invoke("overview")` 之类的命令，不知道也不需要知道数据是
//! 从同进程的 `Store` 读出来的，还是从一台常驻 host 上取回来的。桌面与移动
//! 的全部差异被这一层吸收。
//!
//! 方法面刻意与 `taxweb` 的路由一一对应——它们是同一个东西的两个前端，
//! 包括 `approve` / `reject`：**app 就是人工确认闸口**，和 taxweb 承担同一个角色。
//! MCP 层依然没有这两个能力。

mod local;
mod remote;

pub use local::LocalBackend;
pub use remote::RemoteBackend;

use std::path::Path;

use serde_json::Value;

/// 按扩展名猜 mime。身份是内容哈希、解读看 mime，所以猜错的代价是
/// 后续读取阶段多一次判断，不是数据错乱——认不出的落到 octet-stream。
///
/// 两个 backend 共用：本地直接塞进 `IncomingFile`，远程放进 `Content-Type`。
pub(crate) fn mime_for(path: &Path) -> String {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "pdf" => "application/pdf",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "heic" => "image/heic",
        "webp" => "image/webp",
        "csv" => "text/csv",
        "txt" => "text/plain",
        _ => "application/octet-stream",
    }
    .to_string()
}

/// 错误一律降为字符串：它们最终要过 IPC 到 JS，结构化没有意义。
pub type BackendResult = Result<Value, String>;

pub trait Backend: Send {
    fn overview(&mut self) -> BackendResult;

    /// `date` 缺省为今天；`frequency` 缺省为规则文件里的默认申报频率。
    fn gst(&mut self, date: Option<String>, frequency: Option<String>) -> BackendResult;

    fn ir3(&mut self, year: String) -> BackendResult;

    /// 收一份文档进来。只造 `PendingExtraction`，不产生任何账。
    /// 内容寻址去重：同样的字节进来第二次不会写第二份。
    fn ingest_document(&mut self, path: String) -> BackendResult;

    /// 一份文档，连同记录在它身上的所有 extraction（新版本在前）。
    /// `local_path` 只在本地模式下有值——远程模式下那个文件在 host 上，不在这台机器上。
    fn document(&mut self, document_id: String) -> BackendResult;

    /// 人工决定：忽略一份文档，或把忽略过的放回待提取队列。
    /// 只有这两个迁移——「已提取」是读过文档的东西才能声明的，不是点一下按钮。
    fn set_document_status(&mut self, document_id: String, to: String) -> BackendResult;

    /// 人工确认闸口。只有人点击才会走到这里。
    fn approve_entry(&mut self, entry_id: String) -> BackendResult;

    fn reject_entry(&mut self, entry_id: String) -> BackendResult;
}

/// 后端起不来时的占位（账本打不开、host 没配）。
///
/// 存在的理由是：一个空白窗口比一条明确的报错糟糕得多。宁可让应用正常启动、
/// 每次请求都回同一句原因，也不要在 setup 里 panic。
pub struct UnavailableBackend {
    reason: String,
}

impl UnavailableBackend {
    pub fn new(reason: impl Into<String>) -> Self {
        UnavailableBackend {
            reason: reason.into(),
        }
    }
}

impl Backend for UnavailableBackend {
    fn overview(&mut self) -> BackendResult {
        Err(self.reason.clone())
    }
    fn gst(&mut self, _date: Option<String>, _frequency: Option<String>) -> BackendResult {
        Err(self.reason.clone())
    }
    fn ir3(&mut self, _year: String) -> BackendResult {
        Err(self.reason.clone())
    }
    fn ingest_document(&mut self, _path: String) -> BackendResult {
        Err(self.reason.clone())
    }
    fn document(&mut self, _document_id: String) -> BackendResult {
        Err(self.reason.clone())
    }
    fn set_document_status(&mut self, _document_id: String, _to: String) -> BackendResult {
        Err(self.reason.clone())
    }
    fn approve_entry(&mut self, _entry_id: String) -> BackendResult {
        Err(self.reason.clone())
    }
    fn reject_entry(&mut self, _entry_id: String) -> BackendResult {
        Err(self.reason.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::mime_for;
    use std::path::Path;

    #[test]
    fn mime_falls_back_rather_than_failing() {
        assert_eq!(mime_for(Path::new("a/b.JPG")), "image/jpeg");
        assert_eq!(mime_for(Path::new("statement.csv")), "text/csv");
        assert_eq!(mime_for(Path::new("no-extension")), "application/octet-stream");
    }
}
