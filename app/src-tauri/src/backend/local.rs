//! 直连本地账本。桌面端的默认路径。
//!
//! 没有 HTTP、没有端口、没有独立进程——`Store` 就在这个进程里。
//! 逻辑与 `taxweb::api` 对齐（两者是同一套库的两个前端），偏差应视为 bug。

use std::path::{Path, PathBuf};

use chrono::{NaiveDate, Utc};
use serde_json::json;
use taxcore::{DocumentId, DocumentSource, DocumentStatus, EntryId, EntryStatus, TaxYear};
use taxingest::{IncomingFile, Intake, propose_matches, review_queue};
use taxrules::RuleSet;
use taxstore::Store;

use super::{Backend, BackendResult, mime_for};

pub struct LocalBackend {
    store: Store,
    data_dir: PathBuf,
    rules_dir: PathBuf,
}

impl LocalBackend {
    pub fn open(data_dir: &Path, rules_dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(data_dir)
            .map_err(|e| format!("创建数据目录 {} 失败：{e}", data_dir.display()))?;
        let store = Store::open(data_dir.join("ledger.db"))
            .map_err(|e| format!("打开账本失败：{e}"))?;
        Ok(LocalBackend {
            store,
            data_dir: data_dir.to_path_buf(),
            rules_dir: rules_dir.to_path_buf(),
        })
    }

    fn rules_for(&self, year: TaxYear) -> Result<RuleSet, String> {
        RuleSet::for_year(&self.rules_dir, "NZ", year).map_err(err)
    }
}

impl Backend for LocalBackend {
    fn overview(&mut self) -> BackendResult {
        let queue = review_queue(&self.store).map_err(err)?;
        let mut documents = Vec::new();
        for status in [
            DocumentStatus::PendingExtraction,
            DocumentStatus::Extracted,
            DocumentStatus::NeedsReview,
            DocumentStatus::Ignored,
        ] {
            documents.extend(self.store.documents_with_status(status).map_err(err)?);
        }
        Ok(json!({
            "review_documents": to_value(&queue.documents)?,
            "review_drafts": to_value(&queue.drafts)?,
            "documents": to_value(&documents)?,
            "posted_entries": to_value(&self.store.entries_with_status(EntryStatus::Posted).map_err(err)?)?,
            "unreconciled_bank": to_value(&self.store.unreconciled_bank_txns().map_err(err)?)?,
            "match_candidates": to_value(&propose_matches(&self.store, None).map_err(err)?)?,
            "accounts": to_value(&self.store.accounts(false).map_err(err)?)?,
        }))
    }

    fn gst(&mut self, date: Option<String>, frequency: Option<String>) -> BackendResult {
        let date = match date.as_deref().filter(|s| !s.is_empty()) {
            Some(s) => s
                .parse::<NaiveDate>()
                .map_err(|_| format!("{s} 不是 YYYY-MM-DD 日期"))?,
            None => Utc::now().date_naive(),
        };

        let rules = self.rules_for(TaxYear::containing(date))?;
        let frequency = match frequency.as_deref().filter(|s| !s.is_empty()) {
            None => rules.gst.default_frequency(),
            Some(id) => rules
                .gst
                .frequency(id)
                .ok_or_else(|| format!("未知的申报频率 {id}"))?,
        }
        .frequency()
        .map_err(err)?;
        let period = frequency.period_containing(date);

        // 申报期可能跨税年，此时规则要按期末所在税年取。
        let period_year = TaxYear::containing(period.end);
        let rules = if period_year == TaxYear::containing(date) {
            rules
        } else {
            self.rules_for(period_year)?
        };
        to_value(&taxreturn::gst101(&self.store, &rules, period).map_err(err)?)
    }

    fn ir3(&mut self, year: String) -> BackendResult {
        let year: TaxYear = year.parse().map_err(err)?;
        let rules = self.rules_for(year)?;
        to_value(&taxreturn::ir3(&self.store, &rules, year).map_err(err)?)
    }

    fn ingest_document(&mut self, path: String) -> BackendResult {
        let path = PathBuf::from(path);
        let bytes = std::fs::read(&path).map_err(|e| format!("读取 {} 失败：{e}", path.display()))?;
        let original_filename = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());

        let file = IncomingFile {
            mime: mime_for(&path),
            bytes,
            // 从 app 进来的一律记 Upload。Email / Photo / BankStatement 有各自的入口，
            // 冒充来源会污染 provenance。
            source: DocumentSource::Upload,
            original_filename,
        };

        let intake = taxingest::ingest(&mut self.store, &self.data_dir, file).map_err(err)?;
        let duplicate = matches!(intake, Intake::Duplicate(_));
        Ok(json!({
            "duplicate": duplicate,
            "document": to_value(intake.document())?,
        }))
    }

    fn document(&mut self, document_id: String) -> BackendResult {
        let id: DocumentId = document_id.parse().map_err(err)?;
        let document = self.store.document(id).map_err(err)?;
        // 新版本在前：想看的是最新那次读到了什么，旧版本是历史。
        let mut extractions = self.store.extractions_for(id).map_err(err)?;
        extractions.reverse();

        Ok(json!({
            "document": to_value(&document)?,
            "extractions": to_value(&extractions)?,
            "local_path": self.data_dir.join(&document.stored_path).to_string_lossy(),
        }))
    }

    fn set_document_status(&mut self, document_id: String, to: String) -> BackendResult {
        let id: DocumentId = document_id.parse().map_err(err)?;
        let status = match to.as_str() {
            "ignored" => DocumentStatus::Ignored,
            "pending_extraction" => DocumentStatus::PendingExtraction,
            other => {
                return Err(format!(
                    "{other} 不是人能直接设的状态；只有「忽略」和「放回待提取」是人的决定"
                ));
            }
        };
        self.store.set_document_status(id, status).map_err(err)?;
        Ok(json!({ "ok": true, "status": to_value(&status)? }))
    }

    fn approve_entry(&mut self, entry_id: String) -> BackendResult {
        let id: EntryId = entry_id.parse().map_err(err)?;
        taxingest::approve_draft(&mut self.store, id).map_err(err)?;
        Ok(json!({ "ok": true }))
    }

    fn reject_entry(&mut self, entry_id: String) -> BackendResult {
        let id: EntryId = entry_id.parse().map_err(err)?;
        taxingest::reject_draft(&mut self.store, id).map_err(err)?;
        Ok(json!({ "ok": true }))
    }
}

fn to_value<T: serde::Serialize>(value: &T) -> BackendResult {
    serde_json::to_value(value).map_err(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("financeapp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn ingesting_the_same_bytes_twice_stores_one_document() {
        let dir = tempdir("ingest");
        let mut backend = LocalBackend::open(&dir, &dir.join("rules")).unwrap();

        let invoice = dir.join("INV-8842.pdf");
        std::fs::write(&invoice, b"fake pdf bytes").unwrap();

        let first = backend
            .ingest_document(invoice.to_string_lossy().into_owned())
            .unwrap();
        assert_eq!(first["duplicate"], false);
        // 收进来的文档不产生任何账，只等提取。
        assert_eq!(first["document"]["status"], "pending_extraction");
        assert_eq!(first["document"]["original_filename"], "INV-8842.pdf");
        assert_eq!(first["document"]["mime"], "application/pdf");

        // 换个文件名，同样的字节 —— 身份是内容哈希，所以仍是同一份。
        let renamed = dir.join("copy-of-invoice.pdf");
        std::fs::write(&renamed, b"fake pdf bytes").unwrap();
        let second = backend
            .ingest_document(renamed.to_string_lossy().into_owned())
            .unwrap();
        assert_eq!(second["duplicate"], true);
        assert_eq!(second["document"]["id"], first["document"]["id"]);

        let overview = backend.overview().unwrap();
        assert_eq!(overview["documents"].as_array().unwrap().len(), 1);
        // 收文档不会产生任何草稿或分录。
        assert_eq!(overview["review_drafts"].as_array().unwrap().len(), 0);
        assert_eq!(overview["posted_entries"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn a_document_carries_its_readings_and_a_path_to_the_bytes() {
        let dir = tempdir("doc-detail");
        let mut backend = LocalBackend::open(&dir, &dir.join("rules")).unwrap();

        let invoice = dir.join("INV-9001.pdf");
        std::fs::write(&invoice, b"fake pdf bytes").unwrap();
        let ingested = backend
            .ingest_document(invoice.to_string_lossy().into_owned())
            .unwrap();
        let id = ingested["document"]["id"].as_str().unwrap().to_string();

        let detail = backend.document(id.clone()).unwrap();
        assert_eq!(detail["document"]["original_filename"], "INV-9001.pdf");
        // 还没有 agent 读过它，所以是空的——不是缺字段。
        assert_eq!(detail["extractions"].as_array().unwrap().len(), 0);

        // local_path 必须真的指向那些字节，否则「用系统程序打开」是个死按钮。
        let stored = detail["local_path"].as_str().unwrap();
        assert_eq!(std::fs::read(stored).unwrap(), b"fake pdf bytes");
    }

    #[test]
    fn only_the_two_human_decisions_are_settable() {
        let dir = tempdir("doc-status");
        let mut backend = LocalBackend::open(&dir, &dir.join("rules")).unwrap();

        let receipt = dir.join("personal.pdf");
        std::fs::write(&receipt, b"a private receipt").unwrap();
        let id = backend
            .ingest_document(receipt.to_string_lossy().into_owned())
            .unwrap()["document"]["id"]
            .as_str()
            .unwrap()
            .to_string();

        backend
            .set_document_status(id.clone(), "ignored".into())
            .unwrap();
        assert_eq!(backend.document(id.clone()).unwrap()["document"]["status"], "ignored");

        // 「已提取」是读过文档的东西才能声明的，不能从界面点出来。
        let err = backend
            .set_document_status(id.clone(), "extracted".into())
            .unwrap_err();
        assert!(err.contains("extracted"), "错误里应当点名这个状态，实际 {err}");

        backend
            .set_document_status(id.clone(), "pending_extraction".into())
            .unwrap();
        assert_eq!(
            backend.document(id).unwrap()["document"]["status"],
            "pending_extraction"
        );
    }

    #[test]
    fn a_missing_file_is_an_error_not_a_panic() {
        let dir = tempdir("ingest-missing");
        let mut backend = LocalBackend::open(&dir, &dir.join("rules")).unwrap();
        let err = backend.ingest_document("/nope/not-here.pdf".into()).unwrap_err();
        assert!(err.contains("not-here.pdf"), "错误里应当带上路径，实际 {err}");
    }

}


fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
