use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub singbox: SingboxConfig,
    pub db:      DbConfig,
    pub stats:   StatsConfig,
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

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            singbox: SingboxConfig {
                config_path: "/etc/sing-box/config.json".into(),
                binary_path: "/usr/local/bin/sing-box".into(),
                grpc_addr:   "127.0.0.1:18080".into(),
            },
            db:    DbConfig { path: "/var/lib/sing-box-manager/manager.db".into() },
            stats: StatsConfig { sync_interval_secs: 30, quota_alert_percent: 80 },
        }
    }
}
