//! 布局文档的持久化。
//!
//! 存在 **独立于 `ledger.db` 的 `ui.db`** 里：账本是受审计的不可变记录，
//! UI 偏好不是，两者不该共用一个库文件。而且移动端是瘦客户端、本地没有账本，
//! 但布局仍然要能存——它本来就该是分开的。
//!
//! 每次保存追加一个版本而不是原地覆盖，理由和账本一致：出错时能退回去。
//! 这里没有触发器级防篡改，因为改坏一份布局的代价是重排一次页面，
//! 不是伪造一条财务记录。

use std::path::Path;

use rusqlite::Connection;
use serde_json::Value;

pub struct LayoutStore {
    conn: Connection,
}

impl LayoutStore {
    pub fn open(dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("创建应用目录 {} 失败：{e}", dir.display()))?;
        let conn = Connection::open(dir.join("ui.db"))
            .map_err(|e| format!("打开 ui.db 失败：{e}"))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS layout_doc (
                 version    INTEGER PRIMARY KEY AUTOINCREMENT,
                 doc        TEXT NOT NULL,
                 updated_at TEXT NOT NULL
             );",
        )
        .map_err(|e| format!("初始化 ui.db 失败：{e}"))?;
        Ok(LayoutStore { conn })
    }

    /// 最新一版布局。没有任何版本时返回 `None`，由前端落到内置默认布局。
    pub fn load(&self) -> Result<Option<Value>, String> {
        let text: Option<String> = self
            .conn
            .query_row(
                "SELECT doc FROM layout_doc ORDER BY version DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
            .map_err(|e| format!("读取布局失败：{e}"))?;

        match text {
            Some(text) => serde_json::from_str(&text)
                .map(Some)
                .map_err(|e| format!("布局文档不是合法 JSON：{e}")),
            None => Ok(None),
        }
    }

    /// 追加一个版本，返回新版本号。
    pub fn save(&mut self, doc: &Value) -> Result<i64, String> {
        let text = serde_json::to_string(doc).map_err(|e| format!("序列化布局失败：{e}"))?;
        self.conn
            .execute(
                "INSERT INTO layout_doc (doc, updated_at) VALUES (?1, ?2)",
                rusqlite::params![text, chrono::Utc::now().to_rfc3339()],
            )
            .map_err(|e| format!("保存布局失败：{e}"))?;
        Ok(self.conn.last_insert_rowid())
    }
}

#[cfg(test)]
mod tests {
    use super::LayoutStore;
    use serde_json::json;

    #[test]
    fn load_returns_none_before_first_save() {
        let dir = tempdir("layout-empty");
        let store = LayoutStore::open(&dir).unwrap();
        assert!(store.load().unwrap().is_none());
    }

    #[test]
    fn save_appends_versions_and_load_returns_latest() {
        let dir = tempdir("layout-versions");
        let mut store = LayoutStore::open(&dir).unwrap();

        let v1 = store.save(&json!({ "pages": ["a"] })).unwrap();
        let v2 = store.save(&json!({ "pages": ["b"] })).unwrap();
        assert!(v2 > v1);

        assert_eq!(store.load().unwrap().unwrap(), json!({ "pages": ["b"] }));

        // 旧版本仍在，没有被覆盖。
        let count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM layout_doc", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2);
    }

    fn tempdir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("financeapp-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }
}
