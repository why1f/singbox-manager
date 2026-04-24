use anyhow::{anyhow, Result};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::Value;
use crate::model::node::{InboundNode, Protocol};

const SERVER_IP_TTL: Duration = Duration::from_secs(600);
static SERVER_IP_CACHE: OnceLock<Mutex<Option<(Instant, String)>>> = OnceLock::new();

pub fn list_nodes(cfg: &Value) -> Vec<InboundNode> {
    let Some(arr) = cfg["inbounds"].as_array() else { return vec![]; };
    arr.iter().filter_map(|ib| {
        let tag  = ib["tag"].as_str()?.to_string();
        let port = ib["listen_port"].as_u64().unwrap_or(0) as u16;
        let default_name = default_user_name_for_inbound(ib);
        let uc = ib["users"].as_array().map(|u| {
            u.iter().filter(|item| item["name"].as_str() != Some(default_name)).count()
        }).unwrap_or(0);
        Some(InboundNode { tag, protocol: detect(ib), listen_port: port, user_count: uc })
    }).collect()
}

fn default_user_name_for_inbound(ib: &Value) -> &'static str {
    match ib["type"].as_str().unwrap_or("") {
        "hysteria2" => "hy2-default",
        "tuic"      => "tuic-default",
        "anytls"    => "anytls-default",
        _           => "default",
    }
}

fn detect(ib: &Value) -> Protocol {
    match ib["type"].as_str().unwrap_or("") {
        "vless" => {
            if ib["tls"]["reality"]["enabled"].as_bool() == Some(true) { Protocol::VlessReality }
            else if ib["transport"]["type"].as_str() == Some("ws")     { Protocol::VlessWs }
            else { Protocol::Unknown }
        }
        "vmess"       => {
            if ib["transport"]["type"].as_str() == Some("ws") { Protocol::VmessWs }
            else { Protocol::Unknown }
        }
        "shadowsocks" => Protocol::Shadowsocks,
        "trojan"      => Protocol::Trojan,
        "tuic"        => Protocol::Tuic,
        "anytls"      => Protocol::Anytls,
        "hysteria2"   => Protocol::Hysteria2,
        _             => Protocol::Unknown,
    }
}

/// 优先级：public_base 主机 > 请求 Host > 公网 IP 探测。
pub async fn resolve_server_host(public_base: &str, request_host: Option<&str>) -> Result<String> {
    if let Some(host) = public_base_host(public_base) {
        return Ok(host);
    }
    if let Some(host) = request_host.and_then(authority_host) {
        return Ok(host);
    }
    get_server_ip().await
}

/// 查询本机公网 IP，带超时；失败时返回显式错误。
pub async fn get_server_ip() -> Result<String> {
    if let Some(cached) = SERVER_IP_CACHE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()
        .and_then(|guard| {
            guard.as_ref().and_then(|(ts, ip)| {
                if ts.elapsed() < SERVER_IP_TTL {
                    Some(ip.clone())
                } else {
                    None
                }
            })
        })
    {
        return Ok(cached);
    }

    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .connect_timeout(Duration::from_secs(2))
        .build() else { return Err(anyhow!("构建公网 IP 探测客户端失败")); };
    for url in &["https://api4.ipify.org", "https://ifconfig.me/ip"] {
        if let Ok(resp) = client.get(*url).send().await {
            if let Ok(text) = resp.text().await {
                if let Some(ip) = normalize_server_ip(&text) {
                    if let Ok(mut guard) = SERVER_IP_CACHE.get_or_init(|| Mutex::new(None)).lock() {
                        *guard = Some((Instant::now(), ip.clone()));
                    }
                    return Ok(ip);
                }
            }
        }
    }
    Err(anyhow!("无法探测公网 IP；请配置 subscription.public_base 或通过反代域名访问订阅"))
}

fn public_base_host(public_base: &str) -> Option<String> {
    if public_base.trim().is_empty() {
        return None;
    }
    let rest = public_base
        .trim()
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(public_base.trim());
    authority_host(rest.split('/').next().unwrap_or_default())
}

fn authority_host(authority: &str) -> Option<String> {
    let authority = authority.trim();
    if authority.is_empty() {
        return None;
    }

    if authority.starts_with('[') {
        let end = authority.find(']')?;
        return normalize_server_ip(&authority[..=end]);
    }

    let host = authority.split(':').next().unwrap_or_default().trim();
    normalize_server_ip(host)
}

fn normalize_server_ip(text: &str) -> Option<String> {
    let t = text.trim();
    if t.is_empty() || t.len() > 64 {
        return None;
    }
    // IPv6 地址在 URI 中必须用 [...] 包裹，否则订阅链接格式非法
    Some(if t.contains(':') && !t.starts_with('[') {
        format!("[{}]", t)
    } else {
        t.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::{authority_host, normalize_server_ip, public_base_host};

    #[test]
    fn ipv4_keeps_plain() {
        assert_eq!(normalize_server_ip("1.2.3.4\n"), Some("1.2.3.4".into()));
    }

    #[test]
    fn ipv6_gets_brackets() {
        assert_eq!(normalize_server_ip("2001:db8::1"), Some("[2001:db8::1]".into()));
    }

    #[test]
    fn strips_port_from_domain_authority() {
        assert_eq!(authority_host("sub.example.com:8443"), Some("sub.example.com".into()));
    }

    #[test]
    fn keeps_bracketed_ipv6_without_port() {
        assert_eq!(authority_host("[2001:db8::1]:443"), Some("[2001:db8::1]".into()));
    }

    #[test]
    fn parses_host_from_public_base() {
        assert_eq!(public_base_host("https://sub.example.com/path"), Some("sub.example.com".into()));
    }
}
