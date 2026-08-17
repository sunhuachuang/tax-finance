//! 应用设置：API key、远程 host。
//!
//! 存在 app 数据目录的 `settings.json`，权限 `0600`。**不放进账本目录**——
//! 账本是受审计的不可变记录，设置是应用偏好，两者不该共用一个目录。
//!
//! 为什么是文件而不是 keychain：`ledger.db` 里躺着 7 年的财务记录，本来就是明文。
//! 只给 API key 上锁而账本敞着是安全表演。同一个信任级别、同一套文件权限，才是一致的。
//! 哪天账本上了加密，这里再跟着上 keychain。
//!
//! **key 不会被读回前端。** 它在你输入的那一刻穿过 webview 一次，此后只在 Rust 侧
//! 使用；读设置时返回的是「有没有」和一个掩码提示，不是原文。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Settings {
    /// Claude API key。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anthropic_api_key: Option<String>,
    /// 远程模式的 host，如 `http://mac-mini:5710`。改它需要重启才生效。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finance_host: Option<String>,

    /// 模型服务地址。留空 = Anthropic 官方。
    ///
    /// 填第三方的 **Anthropic 兼容端点** 就能换供应商，协议完全一样：
    /// DeepSeek `https://api.deepseek.com/anthropic`、
    /// Kimi `https://api.moonshot.ai/anthropic`、
    /// GLM `https://api.z.ai/api/anthropic`、
    /// MiniMax `https://api.minimax.io/anthropic`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_base_url: Option<String>,

    /// 模型名。留空 = `claude-opus-5`。换了 base_url 就必须填对应的模型名。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_model: Option<String>,
}

fn path_in(dir: &Path) -> PathBuf {
    dir.join("settings.json")
}

impl Settings {
    /// 读设置。文件不存在、读不动、或内容坏了，都退回空设置——
    /// 一份坏掉的偏好文件不该让应用起不来。
    pub fn load(dir: &Path) -> Self {
        std::fs::read_to_string(path_in(dir))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("创建应用目录 {} 失败：{e}", dir.display()))?;
        let text =
            serde_json::to_string_pretty(self).map_err(|e| format!("序列化设置失败：{e}"))?;
        write_private(&path_in(dir), &text)
    }

    /// 有效的 API key。环境变量优先——开发时 `ANTHROPIC_API_KEY=... npm run tauri dev`
    /// 要能直接盖过存盘的值；打包后的应用读不到 shell 环境，自然走文件。
    pub fn effective_api_key(&self) -> Option<String> {
        env_value("ANTHROPIC_API_KEY").or_else(|| non_empty(self.anthropic_api_key.clone()))
    }

    pub fn api_key_from_env(&self) -> bool {
        env_value("ANTHROPIC_API_KEY").is_some()
    }

    pub fn effective_base_url(&self) -> Option<String> {
        env_value("ANTHROPIC_BASE_URL").or_else(|| non_empty(self.llm_base_url.clone()))
    }

    pub fn effective_model(&self) -> Option<String> {
        non_empty(self.llm_model.clone())
    }

    pub fn effective_host(&self) -> Option<String> {
        env_value("FINANCE_HOST").or_else(|| non_empty(self.finance_host.clone()))
    }

    pub fn host_from_env(&self) -> bool {
        env_value("FINANCE_HOST").is_some()
    }

    /// 给前端看的形态。**不含 key 原文**——只说有没有、从哪来、末尾几位。
    pub fn redacted(&self) -> Value {
        json!({
            "has_api_key": self.effective_api_key().is_some(),
            "api_key_hint": self.effective_api_key().as_deref().map(mask),
            "api_key_from_env": self.api_key_from_env(),
            "finance_host": self.effective_host(),
            "host_from_env": self.host_from_env(),
            "llm_base_url": self.effective_base_url(),
            "llm_model": self.effective_model(),
        })
    }
}

/// 只给自己读写。别人的账户、以及备份时的粗心拷贝，都读不到。
#[cfg(unix)]
fn write_private(path: &Path, text: &str) -> Result<(), String> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    // 权限在创建时就定死，而不是写完再 chmod——那中间有一个窗口文件是 0644。
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("写入 {} 失败：{e}", path.display()))?;
    file.write_all(text.as_bytes())
        .map_err(|e| format!("写入 {} 失败：{e}", path.display()))?;

    // 文件可能是之前用别的权限建的，创建时的 mode 不会生效，补一次。
    let mut perms = file
        .metadata()
        .map_err(|e| format!("读取 {} 权限失败：{e}", path.display()))?
        .permissions();
    if perms.readonly() {
        return Ok(());
    }
    use std::os::unix::fs::PermissionsExt as _;
    perms.set_mode(0o600);
    std::fs::set_permissions(path, perms)
        .map_err(|e| format!("设置 {} 权限失败：{e}", path.display()))
}

/// Windows 上没有直接对应的 mode 位。文件落在用户 profile 下的 AppData，
/// 默认 ACL 已经把其他普通用户挡住了。
#[cfg(not(unix))]
fn write_private(path: &Path, text: &str) -> Result<(), String> {
    std::fs::write(path, text).map_err(|e| format!("写入 {} 失败：{e}", path.display()))
}

fn env_value(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.trim().is_empty())
}

/// `sk-ant-…4f2a`。够认出是哪把 key，不够拿去用。
fn mask(key: &str) -> String {
    let tail: String = key.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    format!("sk-…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("financeapp-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn saving_and_loading_round_trips() {
        let dir = tempdir("roundtrip");
        Settings {
            anthropic_api_key: Some("sk-ant-secret".into()),
            finance_host: Some("http://host:5710".into()),
            llm_base_url: Some("https://api.deepseek.com/anthropic".into()),
            llm_model: Some("deepseek-v4-flash".into()),
        }
        .save(&dir)
        .unwrap();

        let loaded = Settings::load(&dir);
        assert_eq!(loaded.anthropic_api_key.as_deref(), Some("sk-ant-secret"));
        assert_eq!(loaded.finance_host.as_deref(), Some("http://host:5710"));
        assert_eq!(
            loaded.llm_base_url.as_deref(),
            Some("https://api.deepseek.com/anthropic")
        );
        assert_eq!(loaded.llm_model.as_deref(), Some("deepseek-v4-flash"));
    }

    #[cfg(unix)]
    #[test]
    fn the_settings_file_is_not_world_readable() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempdir("perms");
        Settings {
            anthropic_api_key: Some("sk-ant-secret".into()),
            ..Default::default()
        }
        .save(&dir)
        .unwrap();

        let mode = std::fs::metadata(dir.join("settings.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "设置文件里有 API key，不该让别人读到");
    }

    #[cfg(unix)]
    #[test]
    fn an_existing_loose_file_gets_tightened_on_save() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempdir("tighten");
        let path = dir.join("settings.json");
        std::fs::write(&path, "{}").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        Settings {
            anthropic_api_key: Some("sk-ant-secret".into()),
            ..Default::default()
        }
        .save(&dir)
        .unwrap();

        // 创建时的 mode 对已存在的文件不生效，所以保存必须补一次 chmod。
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn a_corrupt_file_falls_back_to_empty_rather_than_failing_startup() {
        let dir = tempdir("corrupt");
        std::fs::write(dir.join("settings.json"), "{ not json").unwrap();
        assert!(Settings::load(&dir).anthropic_api_key.is_none());
    }

    #[test]
    fn the_redacted_view_never_contains_the_key() {
        let settings = Settings {
            anthropic_api_key: Some("sk-ant-api03-abcdef123456".into()),
            ..Default::default()
        };
        let view = settings.redacted().to_string();
        assert!(!view.contains("abcdef123456"), "掩码视图里不该出现 key 原文：{view}");
        assert!(view.contains("3456"), "应当留几位方便认出是哪把 key：{view}");
    }

    #[test]
    fn blank_values_count_as_unset() {
        let settings = Settings {
            anthropic_api_key: Some("   ".into()),
            finance_host: Some(String::new()),
            ..Default::default()
        };
        // 只有在环境变量也没设的时候这个断言才成立。
        if !settings.api_key_from_env() {
            assert!(settings.effective_api_key().is_none());
        }
        if !settings.host_from_env() {
            assert!(settings.effective_host().is_none());
        }
    }
}
