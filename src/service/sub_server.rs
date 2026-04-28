//! Subscription HTTP server bound to the loopback interface (nginx reverse-proxies with TLS).
//!
//! Routes:
//!   GET /sub/:token                        → 按 User-Agent 自动选格式（浏览器→HTML，clash 家族→yaml，其他→base64）
//!   GET /sub/:token?type=sing-box|base64   → 强制 base64 sing-box
//!   GET /sub/:token?type=clash|mihomo|yaml → 强制 mihomo/clash yaml
//!   GET /sub/:token?type=stats             → 强制 HTML 流量统计页

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use serde::Deserialize;
use sqlx::SqlitePool;
use tokio::net::TcpListener;
use tracing::{info, warn};

use crate::{
    core,
    db::user_repo,
    model::{config::AppConfig, user::User},
    service::{node_service, stats_html, sub_service},
};

#[derive(Clone)]
struct SubState {
    pool: SqlitePool,
    cfg: Arc<AppConfig>,
}

#[derive(Deserialize)]
struct SubQuery {
    #[serde(rename = "type")]
    ty: Option<String>,
}

#[derive(Copy, Clone)]
enum Format {
    Stats,
    Yaml,
    Base64,
}

pub async fn run(pool: SqlitePool, cfg: Arc<AppConfig>) -> Result<()> {
    let addr: SocketAddr =
        cfg.subscription.listen.parse().with_context(|| {
            format!("解析 subscription.listen 失败: {}", cfg.subscription.listen)
        })?;

    let state = SubState {
        pool,
        cfg: cfg.clone(),
    };
    let app = Router::new()
        .route("/sub/:token", get(handle_sub))
        .route("/healthz", get(|| async { "ok" }))
        .fallback(|| async { (StatusCode::NOT_FOUND, "") })
        .with_state(state);

    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("订阅服务绑定 {} 失败", addr))?;
    info!("订阅 HTTP 服务监听 {}", addr);
    if let Err(e) = axum::serve(listener, app).await {
        warn!("订阅服务退出: {}", e);
    }
    Ok(())
}

async fn handle_sub(
    State(s): State<SubState>,
    Path(token): Path<String>,
    Query(q): Query<SubQuery>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ua = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let fmt = pick_format(q.ty.as_deref(), ua);

    // token 格式粗校验 + 空串直接 404（revoke 后的账号走这条路）
    if token.is_empty()
        || !token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        || !(16..=64).contains(&token.len())
    {
        return (StatusCode::NOT_FOUND, HeaderMap::new(), Vec::new());
    }

    let user = match user_repo::find_by_token(&s.pool, &token).await {
        Ok(Some(u)) => u,
        _ => return (StatusCode::NOT_FOUND, HeaderMap::new(), Vec::new()),
    };

    let cfg_json = match core::config::load(&s.cfg.singbox.config_path) {
        Ok(v) => v,
        Err(e) => {
            warn!("读取 sing-box 配置失败: {}", e);
            return error_response(fmt, StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
        }
    };

    let request_host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    let addrs = match node_service::resolve_export_server(
        s.cfg.subscription.use_public_base_as_server,
        &s.cfg.subscription.public_base,
        request_host,
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            warn!("解析订阅节点地址失败: {}", e);
            return error_response(fmt, StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
        }
    };

    let (body, ctype): (Vec<u8>, &str) = match fmt {
        Format::Stats => {
            let base = resolve_base_url(&s.cfg.subscription.public_base, &headers);
            let html = stats_html::render(&cfg_json, &user, &addrs, &base);
            (html.into_bytes(), "text/html; charset=utf-8")
        }
        Format::Yaml => match sub_service::generate_clash_yaml(&cfg_json, &user.name, &addrs) {
            Ok(yaml) => (yaml.into_bytes(), "text/yaml; charset=utf-8"),
            Err(e) => {
                return error_response(fmt, StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())
            }
        },
        Format::Base64 => {
            let links = match sub_service::generate_links(&cfg_json, &user.name, &addrs) {
                Ok(links) => links,
                Err(e) => {
                    return error_response(fmt, StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())
                }
            };
            let text = sub_service::generate_subscription(&links);
            (text.into_bytes(), "text/plain; charset=utf-8")
        }
    };

    let mut out = HeaderMap::new();
    out.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static_or_default(ctype),
    );
    if let Ok(v) = HeaderValue::from_str(&userinfo_header(&user)) {
        out.insert(HeaderName::from_static("subscription-userinfo"), v);
    }
    out.insert(
        HeaderName::from_static("profile-update-interval"),
        HeaderValue::from_static("6"),
    );
    // profile-web-page-url 应指向带 token 的订阅统计页，而非裸 public_base
    if !s.cfg.subscription.public_base.is_empty() && !user.sub_token.is_empty() {
        let web_url = format!(
            "{}/sub/{}",
            s.cfg.subscription.public_base.trim_end_matches('/'),
            user.sub_token,
        );
        if let Ok(v) = HeaderValue::from_str(&web_url) {
            out.insert(HeaderName::from_static("profile-web-page-url"), v);
        }
    }
    (StatusCode::OK, out, body)
}

/// 显式 ?type= > UA 嗅探 > 默认 base64
fn pick_format(ty: Option<&str>, ua: &str) -> Format {
    if let Some(t) = ty {
        return match t.to_ascii_lowercase().as_str() {
            "stats" | "html" => Format::Stats,
            "clash" | "mihomo" | "yaml" => Format::Yaml,
            _ => Format::Base64,
        };
    }
    // UA 分流：主流代理客户端的 UA 都带自己的关键字；浏览器几乎必然以 Mozilla 开头
    let u = ua.to_ascii_lowercase();
    if u.starts_with("mozilla/") {
        return Format::Stats;
    }
    if u.contains("clash") || u.contains("mihomo") || u.contains("stash") || u.contains("clashx") {
        return Format::Yaml;
    }
    Format::Base64
}

/// 订阅页里拼完整 URL 用的 base：public_base 优先，否则从 Host 头回退
fn resolve_base_url(public_base: &str, headers: &HeaderMap) -> String {
    if !public_base.is_empty() {
        return public_base.trim_end_matches('/').to_string();
    }
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if host.is_empty() {
        return String::new();
    }
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("https");
    format!("{}://{}", scheme, host)
}

/// subscription-userinfo 头：`upload=X; download=Y; total=Z; expire=T`
fn userinfo_header(u: &User) -> String {
    let mul = u.traffic_multiplier.max(0.0);
    let upload = ((u.used_up_bytes.max(0) as f64) * mul) as u64;
    let download = ((u.used_down_bytes.max(0) as f64) * mul) as u64;
    let total = (u.quota_gb * 1_073_741_824.0) as u64;
    let expire = if u.expire_at.is_empty() {
        0
    } else {
        chrono::NaiveDate::parse_from_str(&u.expire_at, "%Y-%m-%d")
            .ok()
            .and_then(|d| d.and_hms_opt(23, 59, 59))
            .map(|dt| dt.and_utc().timestamp() as u64)
            .unwrap_or(0)
    };
    format!(
        "upload={}; download={}; total={}; expire={}",
        upload, download, total, expire
    )
}

fn error_response(fmt: Format, status: StatusCode, msg: &str) -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut headers = HeaderMap::new();
    let (ctype, body) = match fmt {
        Format::Stats => (
            "text/html; charset=utf-8",
            format!(
                "<!doctype html><html><body><h1>subscription error</h1><pre>{}</pre></body></html>",
                html_escape(msg)
            )
            .into_bytes(),
        ),
        Format::Yaml => (
            "text/plain; charset=utf-8",
            format!("# error: {}\n", msg).into_bytes(),
        ),
        Format::Base64 => (
            "text/plain; charset=utf-8",
            format!("error: {}\n", msg).into_bytes(),
        ),
    };
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static_or_default(ctype),
    );
    (status, headers, body)
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// 用 from_static_or_default 包装：如果静态字符串非 ASCII 就退回 text/plain
trait HvExt {
    fn from_static_or_default(s: &'static str) -> HeaderValue;
}
impl HvExt for HeaderValue {
    fn from_static_or_default(s: &'static str) -> HeaderValue {
        HeaderValue::try_from(s).unwrap_or_else(|_| HeaderValue::from_static("text/plain"))
    }
}

#[cfg(test)]
mod tests {
    use super::{pick_format, userinfo_header, Format};
    use crate::model::user::User;

    fn sample_user() -> User {
        User {
            name: "alice".into(),
            uuid: "de909d94-1d92-4a2f-9da8-c5b52a52282c".into(),
            password: "secret".into(),
            enabled: true,
            quota_gb: 100.0,
            used_up_bytes: 10,
            used_down_bytes: 20,
            last_live_up: 0,
            last_live_down: 0,
            reset_day: 0,
            last_reset_ym: String::new(),
            expire_at: String::new(),
            allow_all_nodes: true,
            created_at: "2026-01-01".into(),
            allowed_nodes: "[]".into(),
            sub_token: "token".into(),
            traffic_multiplier: 2.0,
            tg_chat_id: 0,
            tg_bind_token: String::new(),
            tg_notify_quota_80: true,
            tg_notify_quota_90: true,
            tg_notify_quota_100: true,
            tg_schedule_enabled: true,
            tg_schedule_times: "[]".into(),
            tg_last_quota_level: 0,
            tg_last_schedule_dates: "{}".into(),
        }
    }

    #[test]
    fn browser_defaults_to_stats() {
        assert!(matches!(pick_format(None, "Mozilla/5.0"), Format::Stats));
    }

    #[test]
    fn clash_ua_defaults_to_yaml() {
        assert!(matches!(pick_format(None, "clash-verge/1.0"), Format::Yaml));
    }

    #[test]
    fn userinfo_uses_effective_metered_bytes() {
        let header = userinfo_header(&sample_user());
        assert!(header.contains("upload=20"));
        assert!(header.contains("download=40"));
    }
}
