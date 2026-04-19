use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub singbox: SingboxConfig,
    pub db:      DbConfig,
    pub stats:   StatsConfig,
    #[serde(default)]
    pub kernel:  KernelConfig,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SingboxConfig {
    pub config_path: String,
    pub binary_path: String,
    pub grpc_addr: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbConfig { pub path: String }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsConfig { pub sync_interval_secs: u64, pub quota_alert_percent: u8 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KernelConfig {
    /// 拉取自编译 sing-box (with_v2ray_api) 的 GitHub 仓库，owner/repo 形式。
    #[serde(default = "default_update_repo")]
    pub update_repo: String,
}
impl Default for KernelConfig {
    fn default() -> Self { Self { update_repo: default_update_repo() } }
}
fn default_update_repo() -> String { "why1f/singbox-manager".into() }

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            singbox: SingboxConfig {
                config_path: "/etc/sing-box/config.json".into(),
                binary_path: "/usr/local/bin/sing-box".into(),
                grpc_addr:   "127.0.0.1:18080".into(),
            },
            db:     DbConfig { path: "/var/lib/sing-box-manager/manager.db".into() },
            stats:  StatsConfig { sync_interval_secs: 30, quota_alert_percent: 80 },
            kernel: KernelConfig::default(),
        }
    }
}
