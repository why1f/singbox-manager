//! Subscription HTTP server bound to the loopback interface (nginx reverse-proxies with TLS).
//!
//! Routes:
//!   GET /sub/:token                        → sing-box base64 subscription (default)
//!   GET /sub/:token?type=sing-box|base64   → same as default
//!   GET /sub/:token?type=clash|mihomo      → Clash/Mihomo YAML

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
    core, db::user_repo, model::{config::AppConfig, user::User},
    service::{node_service, sub_service},
};

#[derive(Clone)]
struct SubState {
    pool: SqlitePool,
    cfg:  Arc<AppConfig>,
}

#[derive(Deserialize)]
struct SubQuery {
    #[serde(rename = "type")]
    ty: Option<String>,
}

pub async fn run(pool: SqlitePool, cfg: Arc<AppConfig>) -> Result<()> {
    let addr: SocketAddr = cfg.subscription.listen.parse()
        .with_context(|| format!("解析 subscription.listen 失败: {}", cfg.subscription.listen))?;

    let state = SubState { pool, cfg: cfg.clone() };
    let app = Router::new()
        .route("/sub/:token", get(handle_sub))
        .route("/healthz",    get(|| async { "ok" }))
        .fallback(|| async { (StatusCode::NOT_FOUND, "") })
        .with_state(state);

    let listener = TcpListener::bind(addr).await
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
) -> impl IntoResponse {
    // token 格式粗校验：URL-safe base64 不含 /、+、=；长度 16-64
    if !token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        || !(16..=64).contains(&token.len()) {
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
            return (StatusCode::INTERNAL_SERVER_ERROR, HeaderMap::new(), Vec::new());
        }
    };

    let server = node_service::get_server_ip().await;
    let ty = q.ty.as_deref().unwrap_or("sing-box").to_ascii_lowercase();

    let (body, ctype): (Vec<u8>, &str) = match ty.as_str() {
        "clash" | "mihomo" | "yaml" => {
            let yaml = sub_service::generate_clash_yaml(&cfg_json, &user.name, &server)
                .unwrap_or_else(|e| format!("# error: {}\n", e));
            (yaml.into_bytes(), "text/yaml; charset=utf-8")
        }
        _ => {
            let links = sub_service::generate_links(&cfg_json, &user.name, &server)
                .unwrap_or_default();
            let text = sub_service::generate_subscription(&links);
            (text.into_bytes(), "text/plain; charset=utf-8")
        }
    };

    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static_or_default(ctype));
    if let Ok(v) = HeaderValue::from_str(&userinfo_header(&user)) {
        headers.insert(HeaderName::from_static("subscription-userinfo"), v);
    }
    headers.insert(
        HeaderName::from_static("profile-update-interval"),
        HeaderValue::from_static("6"),
    );
    headers.insert(
        HeaderName::from_static("profile-web-page-url"),
        HeaderValue::from_str(&s.cfg.subscription.public_base).unwrap_or(HeaderValue::from_static("")),
    );
    (StatusCode::OK, headers, body)
}

/// subscription-userinfo 头：`upload=X; download=Y; total=Z; expire=T`
fn userinfo_header(u: &User) -> String {
    let upload   = u.used_up_bytes.max(0) as u64;
    let download = u.used_down_bytes.max(0) as u64 + u.manual_bytes.max(0) as u64;
    let total    = (u.quota_gb * 1_073_741_824.0) as u64;
    let expire = if u.expire_at.is_empty() {
        0
    } else {
        chrono::NaiveDate::parse_from_str(&u.expire_at, "%Y-%m-%d")
            .ok()
            .and_then(|d| d.and_hms_opt(23, 59, 59))
            .map(|dt| dt.and_utc().timestamp() as u64)
            .unwrap_or(0)
    };
    format!("upload={}; download={}; total={}; expire={}", upload, download, total, expire)
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
