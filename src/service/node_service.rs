use anyhow::{anyhow, Result};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::model::node::{InboundNode, Protocol};
use serde_json::Value;

const SERVER_IP_TTL: Duration = Duration::from_secs(600);
static SERVER_IP_CACHE: OnceLock<Mutex<Option<(Instant, String, String)>>> = OnceLock::new();

#[derive(Debug, Clone)]
pub struct ServerAddresses {
    pub ipv4: String,
    pub ipv6: String,
}

pub fn list_nodes(cfg: &Value) -> Vec<InboundNode> {
    let Some(arr) = cfg["inbounds"].as_array() else {
        return vec![];
    };
    arr.iter()
        .filter_map(|ib| {
            let tag = ib["tag"].as_str()?.to_string();
            let port = ib["listen_port"].as_u64().unwrap_or(0) as u16;
            let default_name = default_user_name_for_inbound(ib);
            let uc = ib["users"]
                .as_array()
                .map(|u| {
                    u.iter()
                        .filter(|item| item["name"].as_str() != Some(default_name))
                        .count()
                })
                .unwrap_or(0);
            Some(InboundNode {
                tag,
                protocol: detect(ib),
                listen_port: port,
                user_count: uc,
            })
        })
        .collect()
}

fn default_user_name_for_inbound(ib: &Value) -> &'static str {
    match ib["type"].as_str().unwrap_or("") {
        "hysteria2" => "hy2-default",
        "tuic" => "tuic-default",
        "anytls" => "anytls-default",
        _ => "default",
    }
}

fn detect(ib: &Value) -> Protocol {
    match ib["type"].as_str().unwrap_or("") {
        "vless" => {
            if ib["tls"]["reality"]["enabled"].as_bool() == Some(true) {
                Protocol::VlessReality
            } else if ib["transport"]["type"].as_str() == Some("ws") {
                Protocol::VlessWs
            } else {
                Protocol::Unknown
            }
        }
        "vmess" => {
            if ib["transport"]["type"].as_str() == Some("ws") {
                Protocol::VmessWs
            } else {
                Protocol::Unknown
            }
        }
        "shadowsocks" => Protocol::Shadowsocks,
        "trojan" => Protocol::Trojan,
        "tuic" => Protocol::Tuic,
        "anytls" => Protocol::Anytls,
        "hysteria2" => Protocol::Hysteria2,
        _ => Protocol::Unknown,
    }
}

/// 优先级：public_base 主机 > 请求 Host > 公网 IP 探测。
pub async fn resolve_server_host(public_base: &str, request_host: Option<&str>) -> Result<ServerAddresses> {
    if let Some(host) = public_base_host(public_base) {
        return Ok(ServerAddresses {
            ipv4: host.clone(),
            ipv6: host,
        });
    }
    if let Some(host) = request_host.and_then(authority_host) {
        return Ok(ServerAddresses {
            ipv4: host.clone(),
            ipv6: host,
        });
    }
    get_server_ips().await
}

/// 导出订阅时节点 server 的优先级：
/// 1. 若启用 use_public_base_as_server，则先取 public_base 主机
/// 2. 公网 IP 探测
/// 3. public_base 主机
/// 4. 请求 Host
pub async fn resolve_export_server(
    use_public_base_as_server: bool,
    public_base: &str,
    request_host: Option<&str>,
) -> Result<ServerAddresses> {
    if use_public_base_as_server {
        if let Some(host) = public_base_host(public_base) {
            return Ok(ServerAddresses {
                ipv4: host.clone(),
                ipv6: host,
            });
        }
    }
    if let Ok(addrs) = get_server_ips().await {
        return Ok(addrs);
    }
    if let Some(host) = public_base_host(public_base) {
        return Ok(ServerAddresses {
            ipv4: host.clone(),
            ipv6: host,
        });
    }
    if let Some(host) = request_host.and_then(authority_host) {
        return Ok(ServerAddresses {
            ipv4: host.clone(),
            ipv6: host,
        });
    }
    Err(anyhow!(
        "无法解析订阅导出节点地址；请检查公网 IP 探测或配置 subscription.public_base"
    ))
}

/// 并行查询本机 IPv4 和 IPv6 公网地址，每个有 3s 超时。
/// 两个都失败时返回错误；仅一个成功时另一个回落为成功的那一个。
pub async fn get_server_ips() -> Result<ServerAddresses> {
    if let Some(cached) = SERVER_IP_CACHE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()
        .and_then(|guard| {
            guard.as_ref().and_then(|(ts, v4, v6)| {
                if ts.elapsed() < SERVER_IP_TTL {
                    Some((v4.clone(), v6.clone()))
                } else {
                    None
                }
            })
        })
    {
        return Ok(ServerAddresses {
            ipv4: cached.0,
            ipv6: cached.1,
        });
    }

    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .connect_timeout(Duration::from_secs(2))
        .build()
    else {
        return Err(anyhow!("构建公网 IP 探测客户端失败"));
    };

    let (v4, v6) = tokio::join!(
        fetch_ip(&client, "https://api4.ipify.org"),
        fetch_ip(&client, "https://api6.ipify.org"),
    );

    let v4 = match v4 {
        Some(ip) => ip,
        None => match &v6 {
            Some(ip) => ip.clone(),
            None => {
                // 两个都没拿到，试 ifconfig.me 做最后兜底
                match fetch_ip(&client, "https://ifconfig.me/ip").await {
                    Some(ip) => ip,
                    None => {
                        return Err(anyhow!(
                            "无法探测公网 IP；请配置 subscription.public_base 或通过反代域名访问订阅"
                        ))
                    }
                }
            }
        }
    };
    let v6 = v6.unwrap_or_else(|| v4.clone());

    if let Ok(mut guard) = SERVER_IP_CACHE.get_or_init(|| Mutex::new(None)).lock() {
        *guard = Some((Instant::now(), v4.clone(), v6.clone()));
    }
    Ok(ServerAddresses { ipv4: v4, ipv6: v6 })
}

async fn fetch_ip(client: &reqwest::Client, url: &str) -> Option<String> {
    let resp = client.get(url).send().await.ok()?;
    let text = resp.text().await.ok()?;
    normalize_server_ip(&text)
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

/// 根据节点 meta 选择 IPv4 或 IPv6 地址
pub fn pick_server<'a>(addrs: &'a ServerAddresses, tag: &str) -> &'a str {
    let ipv6 = crate::core::config::get_node_meta(tag)
        .map(|m| m.ipv6)
        .unwrap_or(false);
    if ipv6 {
        &addrs.ipv6
    } else {
        &addrs.ipv4
    }
}

#[cfg(test)]
mod tests {
    use super::{authority_host, normalize_server_ip, public_base_host, resolve_export_server};

    #[test]
    fn ipv4_keeps_plain() {
        assert_eq!(normalize_server_ip("1.2.3.4\n"), Some("1.2.3.4".into()));
    }

    #[test]
    fn ipv6_gets_brackets() {
        assert_eq!(
            normalize_server_ip("2001:db8::1"),
            Some("[2001:db8::1]".into())
        );
    }

    #[test]
    fn strips_port_from_domain_authority() {
        assert_eq!(
            authority_host("sub.example.com:8443"),
            Some("sub.example.com".into())
        );
    }

    #[test]
    fn keeps_bracketed_ipv6_without_port() {
        assert_eq!(
            authority_host("[2001:db8::1]:443"),
            Some("[2001:db8::1]".into())
        );
    }

    #[test]
    fn parses_host_from_public_base() {
        assert_eq!(
            public_base_host("https://sub.example.com/path"),
            Some("sub.example.com".into())
        );
    }

    #[tokio::test]
    async fn export_server_defaults_to_public_base_when_enabled() {
        let addrs = resolve_export_server(true, "https://sub.example.com", Some("x"))
            .await
            .unwrap();
        assert_eq!(addrs.ipv4, "sub.example.com");
        assert_eq!(addrs.ipv6, "sub.example.com");
    }
}
