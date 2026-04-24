use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use sqlx::sqlite::SqlitePoolOptions;
use std::path::Path;

use crate::{core, model::config::AppConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckLevel {
    Ok,
    Warn,
    Error,
}

#[derive(Debug, Clone)]
pub struct CheckItem {
    pub level: CheckLevel,
    pub label: &'static str,
    pub detail: String,
}

#[derive(Debug, Default, Clone)]
pub struct DoctorReport {
    pub items: Vec<CheckItem>,
}

impl DoctorReport {
    pub fn push(&mut self, level: CheckLevel, label: &'static str, detail: impl Into<String>) {
        self.items.push(CheckItem {
            level,
            label,
            detail: detail.into(),
        });
    }

    pub fn ok_count(&self) -> usize {
        self.items
            .iter()
            .filter(|i| i.level == CheckLevel::Ok)
            .count()
    }

    pub fn warn_count(&self) -> usize {
        self.items
            .iter()
            .filter(|i| i.level == CheckLevel::Warn)
            .count()
    }

    pub fn error_count(&self) -> usize {
        self.items
            .iter()
            .filter(|i| i.level == CheckLevel::Error)
            .count()
    }

    pub fn has_errors(&self) -> bool {
        self.error_count() > 0
    }
}

pub async fn run(config_path: &Path, cfg: &AppConfig) -> DoctorReport {
    let mut report = DoctorReport::default();

    if config_path.exists() {
        report.push(
            CheckLevel::Ok,
            "配置文件",
            format!("已读取 {}", config_path.display()),
        );
    } else {
        report.push(
            CheckLevel::Error,
            "配置文件",
            format!("不存在: {}", config_path.display()),
        );
    }

    match check_database(&cfg.db.path).await {
        Ok(()) => report.push(CheckLevel::Ok, "数据库", format!("可读写 {}", cfg.db.path)),
        Err(e) => report.push(CheckLevel::Error, "数据库", e.to_string()),
    }

    let kernel_status = core::singbox::status();
    if Path::new(&cfg.singbox.binary_path).exists() {
        let version = kernel_status.version.as_deref().unwrap_or("版本未知");
        report.push(
            CheckLevel::Ok,
            "sing-box 二进制",
            format!("{} ({})", cfg.singbox.binary_path, version),
        );
    } else if let Some(found) = kernel_status.binary_path.as_deref() {
        report.push(
            CheckLevel::Warn,
            "sing-box 二进制",
            format!(
                "配置路径不存在: {}，但系统找到了 {}",
                cfg.singbox.binary_path, found
            ),
        );
    } else {
        report.push(
            CheckLevel::Error,
            "sing-box 二进制",
            format!("未找到 {}", cfg.singbox.binary_path),
        );
    }

    match inspect_singbox_config(cfg) {
        Ok(config) => {
            report.push(
                CheckLevel::Ok,
                "sing-box 配置",
                format!("已解析 {}", cfg.singbox.config_path),
            );

            if Path::new(&cfg.singbox.binary_path).exists() {
                let proc = core::singbox::SingboxProcess::new(
                    &cfg.singbox.binary_path,
                    &cfg.singbox.config_path,
                );
                match proc.check_config() {
                    Ok(()) => report.push(CheckLevel::Ok, "配置校验", "sing-box check 通过"),
                    Err(e) => report.push(CheckLevel::Error, "配置校验", e.to_string()),
                }
            }

            match check_v2ray_api(&config, &cfg.singbox.grpc_addr) {
                Ok(detail) => report.push(CheckLevel::Ok, "v2ray_api", detail),
                Err(e) => report.push(CheckLevel::Error, "v2ray_api", e.to_string()),
            }

            match collect_tls_issues(&config) {
                Ok(issues) if issues.is_empty() => {
                    report.push(CheckLevel::Ok, "证书文件", "TLS 引用的证书/密钥文件存在");
                }
                Ok(issues) => {
                    report.push(CheckLevel::Error, "证书文件", issues.join("；"));
                }
                Err(e) => report.push(CheckLevel::Error, "证书文件", e.to_string()),
            }
        }
        Err(e) => report.push(CheckLevel::Error, "sing-box 配置", e.to_string()),
    }

    match core::grpc::connect(&cfg.singbox.grpc_addr).await {
        Ok(_) => report.push(
            CheckLevel::Ok,
            "gRPC",
            format!("已连接 {}", cfg.singbox.grpc_addr),
        ),
        Err(e) => {
            let level = if kernel_status.running == Some(true) {
                CheckLevel::Error
            } else {
                CheckLevel::Warn
            };
            let detail = if kernel_status.running == Some(true) {
                format!(
                    "{}；如果你装的是官方 sing-box，通常是没启用 with_v2ray_api",
                    e
                )
            } else {
                format!("{}；当前 sing-box 未运行或尚未启动", e)
            };
            report.push(level, "gRPC", detail);
        }
    }

    match check_subscription(cfg) {
        Ok(detail) => report.push(CheckLevel::Ok, "订阅配置", detail),
        Err(e) => report.push(CheckLevel::Warn, "订阅配置", e.to_string()),
    }

    match check_nginx(cfg) {
        Ok((level, detail)) => report.push(level, "nginx", detail),
        Err(e) => report.push(CheckLevel::Error, "nginx", e.to_string()),
    }

    report
}

async fn check_database(path: &str) -> Result<()> {
    if !Path::new(path).exists() {
        return Err(anyhow!("数据库文件不存在: {}", path));
    }
    let url = format!("sqlite://{}?mode=rw", path);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .with_context(|| format!("打开数据库 {} 失败", path))?;
    sqlx::query("SELECT 1")
        .execute(&pool)
        .await
        .context("执行 SELECT 1 失败")?;
    Ok(())
}

fn inspect_singbox_config(cfg: &AppConfig) -> Result<Value> {
    if !Path::new(&cfg.singbox.config_path).exists() {
        return Err(anyhow!("config.json 不存在: {}", cfg.singbox.config_path));
    }
    core::config::load(&cfg.singbox.config_path)
}

fn check_v2ray_api(config: &Value, grpc_addr: &str) -> Result<String> {
    let api = config
        .get("experimental")
        .and_then(|v| v.get("v2ray_api"))
        .ok_or_else(|| anyhow!("缺少 experimental.v2ray_api 配置"))?;
    let listen = api
        .get("listen")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("experimental.v2ray_api.listen 缺失"))?;
    if listen != grpc_addr {
        return Err(anyhow!(
            "v2ray_api.listen={} 与 config.toml.grpc_addr={} 不一致",
            listen,
            grpc_addr
        ));
    }
    let enabled = api
        .get("stats")
        .and_then(|v| v.get("enabled"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !enabled {
        return Err(anyhow!("experimental.v2ray_api.stats.enabled 未开启"));
    }
    let users = api
        .get("stats")
        .and_then(|v| v.get("users"))
        .and_then(Value::as_array)
        .map(|v| v.len())
        .unwrap_or(0);
    Ok(format!(
        "listen={}，stats 已启用，当前 {} 个统计用户",
        listen, users
    ))
}

fn collect_tls_issues(config: &Value) -> Result<Vec<String>> {
    let mut issues = Vec::new();
    let Some(inbounds) = config.get("inbounds").and_then(Value::as_array) else {
        return Ok(issues);
    };

    for inbound in inbounds {
        let tag = inbound
            .get("tag")
            .and_then(Value::as_str)
            .unwrap_or("<unknown>");
        let tls = inbound.get("tls").and_then(Value::as_object);
        let Some(tls) = tls else { continue };

        if tls.get("enabled").and_then(Value::as_bool) != Some(true) {
            continue;
        }
        if tls
            .get("reality")
            .and_then(|v| v.get("enabled"))
            .and_then(Value::as_bool)
            == Some(true)
        {
            continue;
        }
        if tls.get("acme").is_some() {
            continue;
        }

        match tls.get("certificate_path").and_then(Value::as_str) {
            Some(path) if Path::new(path).exists() => {}
            Some(path) => issues.push(format!("{} 证书不存在: {}", tag, path)),
            None => issues.push(format!("{} 缺少 certificate_path", tag)),
        }
        match tls.get("key_path").and_then(Value::as_str) {
            Some(path) if Path::new(path).exists() => {}
            Some(path) => issues.push(format!("{} 私钥不存在: {}", tag, path)),
            None => issues.push(format!("{} 缺少 key_path", tag)),
        }
    }

    Ok(issues)
}

fn check_subscription(cfg: &AppConfig) -> Result<String> {
    if !cfg.subscription.enabled {
        return Ok("订阅服务已关闭".into());
    }
    cfg.subscription
        .listen
        .parse::<std::net::SocketAddr>()
        .with_context(|| format!("listen 不是合法地址: {}", cfg.subscription.listen))?;

    if cfg.subscription.public_base.trim().is_empty() {
        return Err(anyhow!(
            "已启用订阅服务，但 public_base 为空；token 可用，完整 URL 无法自动拼接"
        ));
    }
    let host = parse_public_base(&cfg.subscription.public_base)?;
    Ok(format!(
        "listen={}，public_base 主机={}",
        cfg.subscription.listen, host
    ))
}

fn check_nginx(cfg: &AppConfig) -> Result<(CheckLevel, String)> {
    let status = core::nginx::status(&cfg.subscription.nginx_conf);
    if !status.installed {
        return Ok((CheckLevel::Warn, "未检测到 nginx".into()));
    }
    if !Path::new(&cfg.subscription.nginx_conf).exists() {
        return Ok((
            CheckLevel::Warn,
            format!("配置文件不存在: {}", cfg.subscription.nginx_conf),
        ));
    }
    match core::nginx::test_config() {
        Ok(_) => Ok((
            CheckLevel::Ok,
            format!(
                "已安装，nginx -t 通过，conf={}",
                cfg.subscription.nginx_conf
            ),
        )),
        Err(e) => Ok((
            CheckLevel::Error,
            format!("nginx 已安装但配置校验失败: {}", e),
        )),
    }
}

fn parse_public_base(public_base: &str) -> Result<String> {
    let trimmed = public_base.trim();
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err(anyhow!(
            "public_base 必须以 http:// 或 https:// 开头: {}",
            public_base
        ));
    }
    let rest = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or_default();
    let host = rest.split('/').next().unwrap_or_default().trim();
    if host.is_empty() {
        return Err(anyhow!("public_base 缺少主机名: {}", public_base));
    }
    Ok(host.into())
}
