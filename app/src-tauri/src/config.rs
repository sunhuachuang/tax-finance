//! 启动配置：这个进程是直连本地账本，还是当一台常驻 host 的瘦客户端。
//!
//! 桌面默认本地——账本就在这台机器上，同进程直读。移动端默认远程——
//! 手机上不存在 `~/.taxdata`，也刻意不做本地副本（见 ARCHITECTURE.md
//! 「为什么移动端是瘦客户端」）。

use std::path::PathBuf;

use crate::settings::Settings;

/// 数据从哪来。`Backend` 的两个实现各对应一支。
pub enum Mode {
    /// 直连本地 `ledger.db`。
    Local {
        data_dir: PathBuf,
        rules_dir: PathBuf,
    },
    /// 打到常驻 host 的 `taxweb`，例如 `http://mac-mini:5710`。
    /// 明文 HTTP 是可以的：这条链路只在 Tailscale / WireGuard 隧道里跑，
    /// 隧道本身已经加密且做了设备认证。绝不要把它暴露到公网。
    Remote { base_url: String },
}

impl Mode {
    /// 按设置和环境变量决定，其次按平台取默认值。
    ///
    /// host 从 `Settings` 来（那里已经处理了「环境变量优先于设置文件」）；
    /// 数据和规则目录目前仍只看环境变量——打包后的应用两者都有更好的出路
    /// （规则打进 bundle、数据用默认目录），不必再暴露成设置项。
    pub fn resolve(settings: &Settings) -> Self {
        if let Some(host) = settings.effective_host() {
            return Mode::Remote {
                base_url: host.trim().trim_end_matches('/').to_string(),
            };
        }

        if cfg!(mobile) {
            // 移动端没有可用的本地账本。没配 host 就报错，而不是静默造一个空账本
            // ——空账本会让人以为「数据没了」，比一条明确的报错糟糕得多。
            return Mode::Remote {
                base_url: String::new(),
            };
        }

        let data_dir = env_path("FINANCE_DATA_DIR").unwrap_or_else(default_data_dir);
        let rules_dir = env_path("FINANCE_RULES_DIR").unwrap_or_else(|| data_dir.join("rules"));
        Mode::Local {
            data_dir,
            rules_dir,
        }
    }

    /// 展示给 UI 的一行描述，用于状态栏和排错。
    pub fn describe(&self) -> String {
        match self {
            Mode::Local { data_dir, .. } => format!("本地账本 {}", data_dir.display()),
            Mode::Remote { base_url } if base_url.is_empty() => "未配置 host".to_string(),
            Mode::Remote { base_url } => format!("远程 {base_url}"),
        }
    }
}

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var(key).ok().filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// 与 `taxweb` / `taxmcp` 保持同一个默认目录，三个进程共享一份账本。
fn default_data_dir() -> PathBuf {
    match std::env::var("HOME") {
        Ok(home) => PathBuf::from(home).join(".taxdata"),
        Err(_) => PathBuf::from(".taxdata"),
    }
}
