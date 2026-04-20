use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub singbox: SingboxConfig,
    pub db:      DbConfig,
    pub stats:   StatsConfig,
    #[serde(default)]
    pub kernel:  KernelConfig,
    #[serde(default)]
    pub subscription: SubscriptionConfig,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionConfig {
    /// 订阅 HTTP 服务本地监听地址（nginx 反代上游）
    #[serde(default = "default_sub_listen")]
    pub listen: String,
    /// 对外公开的基础 URL（用于拼订阅链接）。不设置则只输出 token，由管理员自己拼
    #[serde(default)]
    pub public_base: String,
    /// TUI 生成 nginx 配置时写入的文件路径
    #[serde(default = "default_nginx_conf")]
    pub nginx_conf: String,
    /// 是否启用订阅服务（关闭则 daemon/tui 不起 HTTP 监听）
    #[serde(default = "default_true")]
    pub enabled: bool,
}
impl Default for SubscriptionConfig {
    fn default() -> Self {
        Self {
            listen: default_sub_listen(),
            public_base: String::new(),
            nginx_conf: default_nginx_conf(),
            enabled: true,
        }
    }
}
fn default_sub_listen() -> String { "127.0.0.1:18081".into() }
fn default_nginx_conf() -> String { "/etc/nginx/conf.d/sb-manager.conf".into() }
fn default_true() -> bool { true }

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            singbox: SingboxConfig {
                config_path: "/etc/sing-box/config.json".into(),
                binary_path: "/usr/local/bin/sing-box".into(),
                grpc_addr:   "127.0.0.1:18080".into(),
            },
            db:           DbConfig { path: "/var/lib/sing-box-manager/manager.db".into() },
            stats:        StatsConfig { sync_interval_secs: 30, quota_alert_percent: 80 },
            kernel:       KernelConfig::default(),
            subscription: SubscriptionConfig::default(),
        }
    }
}
