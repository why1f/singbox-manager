use anyhow::Result;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct ShareLink {
    pub tag: String,
    pub protocol: String,
    pub link: String,
}

pub fn generate_links(cfg: &Value, username: &str, server: &str) -> Result<Vec<ShareLink>> {
    let mut links = Vec::new();
    let Some(inbounds) = cfg["inbounds"].as_array() else {
        return Ok(links);
    };
    for ib in inbounds {
        let tag = ib["tag"].as_str().unwrap_or("");
        let typ = ib["type"].as_str().unwrap_or("");
        let port = effective_port(ib, tag);
        let Some(user) = find_user(ib, username) else {
            continue;
        };
        let link = match typ {
            "vless" => {
                if ib["tls"]["reality"]["enabled"].as_bool() == Some(true) {
                    vless_reality(ib, user, server, port, tag)
                } else if ib["transport"]["type"].as_str() == Some("ws") {
                    vless_ws(ib, user, server, port, tag)
                } else {
                    None
                }
            }
            "vmess" => vmess_ws(ib, user, server, port, tag),
            "shadowsocks" => shadowsocks(ib, user, server, port, tag),
            "trojan" => trojan(ib, user, server, port, tag),
            "hysteria2" => hysteria2(ib, user, server, port, tag),
            "tuic" => tuic(ib, user, server, port, tag),
            "anytls" => anytls(ib, user, server, port, tag),
            _ => None,
        };
        if let Some(l) = link {
            links.push(ShareLink {
                tag: tag.into(),
                protocol: typ.into(),
                link: l,
            });
        }
    }
    Ok(links)
}

/// 订阅 URL 里用的端口：端口复用节点回 443，否则读 inbound.listen_port
fn effective_port(ib: &Value, tag: &str) -> u64 {
    let raw = ib["listen_port"].as_u64().unwrap_or(0);
    if crate::core::config::get_node_meta(tag)
        .map(|m| m.port_reuse)
        .unwrap_or(false)
    {
        443
    } else {
        raw
    }
}

pub fn generate_subscription(links: &[ShareLink]) -> String {
    STANDARD.encode(
        links
            .iter()
            .map(|l| l.link.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// 生成 Clash/Mihomo 格式的 YAML 订阅
pub fn generate_clash_yaml(cfg: &Value, username: &str, server: &str) -> Result<String> {
    use std::fmt::Write;
    let mut out = String::new();
    writeln!(out, "# mihomo / clash-meta subscription for {}", username).ok();
    writeln!(out, "mixed-port: 7890").ok();
    writeln!(out, "allow-lan: false").ok();
    writeln!(out, "mode: rule").ok();
    writeln!(out, "log-level: info").ok();
    writeln!(out).ok();
    writeln!(out, "proxies:").ok();

    let mut proxy_names: Vec<String> = Vec::new();
    let Some(inbounds) = cfg["inbounds"].as_array() else {
        writeln!(out, "# no inbounds").ok();
        return Ok(out);
    };

    for ib in inbounds {
        let tag = ib["tag"].as_str().unwrap_or("");
        let typ = ib["type"].as_str().unwrap_or("");
        let port = effective_port(ib, tag);
        let Some(user) = find_user(ib, username) else {
            continue;
        };

        let proxy_name = tag.to_string();
        let added = match typ {
            "vless" => {
                if ib["tls"]["reality"]["enabled"].as_bool() == Some(true) {
                    clash_vless_reality(&mut out, ib, user, server, port, &proxy_name, tag)
                } else if ib["transport"]["type"].as_str() == Some("ws") {
                    clash_vless_ws(&mut out, ib, user, server, port, &proxy_name)
                } else {
                    false
                }
            }
            "vmess" => clash_vmess_ws(&mut out, ib, user, server, port, &proxy_name),
            "shadowsocks" => clash_ss(&mut out, ib, user, server, port, &proxy_name),
            "trojan" => clash_trojan(&mut out, ib, user, server, port, &proxy_name),
            "hysteria2" => clash_hy2(&mut out, ib, user, server, port, &proxy_name),
            "tuic" => clash_tuic(&mut out, ib, user, server, port, &proxy_name),
            "anytls" => clash_anytls(&mut out, ib, user, server, port, &proxy_name),
            _ => false,
        };
        if added {
            proxy_names.push(proxy_name);
        }
    }

    writeln!(out).ok();
    writeln!(out, "proxy-groups:").ok();
    writeln!(out, "  - name: 节点选择").ok();
    writeln!(out, "    type: select").ok();
    writeln!(out, "    proxies:").ok();
    writeln!(out, "      - 自动选择").ok();
    writeln!(out, "      - DIRECT").ok();
    for n in &proxy_names {
        writeln!(out, "      - {}", yaml_str(n)).ok();
    }
    writeln!(out, "  - name: 自动选择").ok();
    writeln!(out, "    type: url-test").ok();
    writeln!(out, "    url: http://www.gstatic.com/generate_204").ok();
    writeln!(out, "    interval: 300").ok();
    writeln!(out, "    proxies:").ok();
    for n in &proxy_names {
        writeln!(out, "      - {}", yaml_str(n)).ok();
    }

    writeln!(out).ok();
    writeln!(out, "rules:").ok();
    writeln!(out, "  - GEOIP,CN,DIRECT").ok();
    writeln!(out, "  - MATCH,节点选择").ok();

    Ok(out)
}

fn yaml_str(s: &str) -> String {
    // 简单 YAML 字符串：如果含有特殊字符就用双引号
    if s.chars()
        .any(|c| matches!(c, ':' | '#' | '\'' | '"' | '\n' | '\t'))
    {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

fn clash_vless_reality(
    out: &mut String,
    ib: &Value,
    user: &Value,
    s: &str,
    p: u64,
    name: &str,
    tag: &str,
) -> bool {
    use std::fmt::Write;
    let Some(uuid) = user["uuid"].as_str() else {
        return false;
    };
    let sni = ib["tls"]["server_name"].as_str().unwrap_or("www.apple.com");
    let pk = crate::core::config::get_node_meta(tag)
        .and_then(|m| m.public_key)
        .unwrap_or_default();
    let sid = ib["tls"]["reality"]["short_id"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(Value::as_str)
        .unwrap_or("");
    writeln!(out, "  - name: {}", yaml_str(name)).ok();
    writeln!(out, "    type: vless").ok();
    writeln!(out, "    server: {}", s).ok();
    writeln!(out, "    port: {}", p).ok();
    writeln!(out, "    uuid: {}", uuid).ok();
    writeln!(out, "    network: tcp").ok();
    writeln!(out, "    udp: true").ok();
    writeln!(out, "    tls: true").ok();
    writeln!(out, "    flow: xtls-rprx-vision").ok();
    writeln!(out, "    servername: {}", yaml_str(sni)).ok();
    writeln!(out, "    reality-opts:").ok();
    writeln!(out, "      public-key: {}", yaml_str(&pk)).ok();
    writeln!(out, "      short-id: {}", yaml_str(sid)).ok();
    writeln!(out, "    client-fingerprint: chrome").ok();
    true
}

fn clash_vless_ws(out: &mut String, ib: &Value, user: &Value, s: &str, p: u64, name: &str) -> bool {
    use std::fmt::Write;
    let Some(uuid) = user["uuid"].as_str() else {
        return false;
    };
    let path = ib["transport"]["path"].as_str().unwrap_or("/");
    let sni = ib["tls"]["server_name"].as_str().unwrap_or(s);
    let tls = ib["tls"]["enabled"].as_bool().unwrap_or(false);
    writeln!(out, "  - name: {}", yaml_str(name)).ok();
    writeln!(out, "    type: vless").ok();
    writeln!(out, "    server: {}", s).ok();
    writeln!(out, "    port: {}", p).ok();
    writeln!(out, "    uuid: {}", uuid).ok();
    writeln!(out, "    network: ws").ok();
    writeln!(out, "    udp: true").ok();
    writeln!(out, "    tls: {}", tls).ok();
    writeln!(out, "    servername: {}", yaml_str(sni)).ok();
    writeln!(out, "    skip-cert-verify: true").ok();
    writeln!(out, "    ws-opts:").ok();
    writeln!(out, "      path: {}", yaml_str(path)).ok();
    true
}

fn clash_vmess_ws(out: &mut String, ib: &Value, user: &Value, s: &str, p: u64, name: &str) -> bool {
    use std::fmt::Write;
    let Some(uuid) = user["uuid"].as_str() else {
        return false;
    };
    let path = ib["transport"]["path"].as_str().unwrap_or("/");
    let sni = ib["tls"]["server_name"].as_str().unwrap_or(s);
    let tls = ib["tls"]["enabled"].as_bool().unwrap_or(false);
    writeln!(out, "  - name: {}", yaml_str(name)).ok();
    writeln!(out, "    type: vmess").ok();
    writeln!(out, "    server: {}", s).ok();
    writeln!(out, "    port: {}", p).ok();
    writeln!(out, "    uuid: {}", uuid).ok();
    writeln!(out, "    alterId: 0").ok();
    writeln!(out, "    cipher: auto").ok();
    writeln!(out, "    network: ws").ok();
    writeln!(out, "    udp: true").ok();
    writeln!(out, "    tls: {}", tls).ok();
    writeln!(out, "    servername: {}", yaml_str(sni)).ok();
    writeln!(out, "    skip-cert-verify: true").ok();
    writeln!(out, "    ws-opts:").ok();
    writeln!(out, "      path: {}", yaml_str(path)).ok();
    true
}

fn clash_ss(out: &mut String, ib: &Value, user: &Value, s: &str, p: u64, name: &str) -> bool {
    use std::fmt::Write;
    // SS inbound users 条目结构为 {name, password}；password 已是 base64(uuid bytes)
    let Some(pw) = user["password"].as_str() else {
        return false;
    };
    let method = ib["method"].as_str().unwrap_or("2022-blake3-aes-128-gcm");
    writeln!(out, "  - name: {}", yaml_str(name)).ok();
    writeln!(out, "    type: ss").ok();
    writeln!(out, "    server: {}", s).ok();
    writeln!(out, "    port: {}", p).ok();
    writeln!(out, "    cipher: {}", yaml_str(method)).ok();
    writeln!(out, "    password: {}", yaml_str(pw)).ok();
    writeln!(out, "    udp: true").ok();
    true
}

fn clash_trojan(out: &mut String, ib: &Value, user: &Value, s: &str, p: u64, name: &str) -> bool {
    use std::fmt::Write;
    let Some(pw) = user["password"].as_str() else {
        return false;
    };
    let sni = ib["tls"]["server_name"].as_str().unwrap_or(s);
    writeln!(out, "  - name: {}", yaml_str(name)).ok();
    writeln!(out, "    type: trojan").ok();
    writeln!(out, "    server: {}", s).ok();
    writeln!(out, "    port: {}", p).ok();
    writeln!(out, "    password: {}", yaml_str(pw)).ok();
    writeln!(out, "    udp: true").ok();
    writeln!(out, "    sni: {}", yaml_str(sni)).ok();
    writeln!(out, "    skip-cert-verify: true").ok();
    true
}

fn clash_hy2(out: &mut String, ib: &Value, user: &Value, s: &str, p: u64, name: &str) -> bool {
    use std::fmt::Write;
    let Some(pw) = user["password"].as_str() else {
        return false;
    };
    let sni = ib["tls"]["server_name"].as_str().unwrap_or(s);
    writeln!(out, "  - name: {}", yaml_str(name)).ok();
    writeln!(out, "    type: hysteria2").ok();
    writeln!(out, "    server: {}", s).ok();
    writeln!(out, "    port: {}", p).ok();
    writeln!(out, "    password: {}", yaml_str(pw)).ok();
    writeln!(out, "    sni: {}", yaml_str(sni)).ok();
    writeln!(out, "    skip-cert-verify: true").ok();
    true
}

fn clash_tuic(out: &mut String, ib: &Value, user: &Value, s: &str, p: u64, name: &str) -> bool {
    use std::fmt::Write;
    let Some(uuid) = user["uuid"].as_str() else {
        return false;
    };
    let Some(pw) = user["password"].as_str() else {
        return false;
    };
    let sni = ib["tls"]["server_name"].as_str().unwrap_or(s);
    writeln!(out, "  - name: {}", yaml_str(name)).ok();
    writeln!(out, "    type: tuic").ok();
    writeln!(out, "    server: {}", s).ok();
    writeln!(out, "    port: {}", p).ok();
    writeln!(out, "    uuid: {}", uuid).ok();
    writeln!(out, "    password: {}", yaml_str(pw)).ok();
    writeln!(out, "    sni: {}", yaml_str(sni)).ok();
    writeln!(out, "    alpn: [h3]").ok();
    writeln!(out, "    congestion-controller: bbr").ok();
    writeln!(out, "    udp-relay-mode: native").ok();
    writeln!(out, "    skip-cert-verify: true").ok();
    true
}

fn clash_anytls(out: &mut String, ib: &Value, user: &Value, s: &str, p: u64, name: &str) -> bool {
    use std::fmt::Write;
    let Some(pw) = user["password"].as_str() else {
        return false;
    };
    let sni = ib["tls"]["server_name"].as_str().unwrap_or(s);
    writeln!(out, "  - name: {}", yaml_str(name)).ok();
    writeln!(out, "    type: anytls").ok();
    writeln!(out, "    server: {}", s).ok();
    writeln!(out, "    port: {}", p).ok();
    writeln!(out, "    password: {}", yaml_str(pw)).ok();
    writeln!(out, "    sni: {}", yaml_str(sni)).ok();
    writeln!(out, "    skip-cert-verify: true").ok();
    true
}

fn find_user<'a>(ib: &'a Value, name: &str) -> Option<&'a Value> {
    ib["users"]
        .as_array()?
        .iter()
        .find(|u| u["name"].as_str() == Some(name))
}

/// 是否在订阅中加 allowInsecure=1：reality 信任 TLS 层，走完整握手；
/// acme 拿的是真实 CA 签的证书；用户自己写入的 cert_path（不在我们托管目录下）也视为真证书。
/// 其他情况——自签证书（cert_path 在我们托管目录下）或根本没 cert_path——客户端都校验不过，必须 insecure。
fn insecure_flag(ib: &Value) -> bool {
    let tls = &ib["tls"];
    if tls["reality"]["enabled"].as_bool() == Some(true) {
        return false;
    }
    if tls["acme"].as_object().is_some() {
        return false;
    }
    match tls["certificate_path"].as_str() {
        None => true,
        Some(p) => p.starts_with(crate::core::config::CERTS_DIR),
    }
}

fn fragment(tag: &str) -> String {
    urlencoding::encode(tag).into_owned()
}

fn vless_reality(ib: &Value, user: &Value, s: &str, p: u64, tag: &str) -> Option<String> {
    let uuid = user["uuid"].as_str()?;
    let sni = ib["tls"]["server_name"].as_str().unwrap_or("www.apple.com");
    let pk = crate::core::config::get_node_meta(tag)
        .and_then(|m| m.public_key)
        .unwrap_or_default();
    let sid = ib["tls"]["reality"]["short_id"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(Value::as_str)
        .unwrap_or("");
    Some(format!(
        "vless://{}@{}:{}?encryption=none&flow=xtls-rprx-vision&security=reality&sni={}&fp=chrome&pbk={}&sid={}&type=tcp#{}",
        uuid, s, p, sni, pk, sid, fragment(tag)))
}

fn vless_ws(ib: &Value, user: &Value, s: &str, p: u64, tag: &str) -> Option<String> {
    let uuid = user["uuid"].as_str()?;
    let path = ib["transport"]["path"].as_str().unwrap_or("/");
    let tls = ib["tls"]["enabled"].as_bool().unwrap_or(false);
    // security 根据 inbound 是否真正启用了 TLS 决定：
    // - TLS 开启（后端直连 TLS）→ security=tls
    // - TLS 未开启（nginx 等前代做终结）→ security=none，客户端明文连 sing-box
    let security = if tls { "tls" } else { "none" };
    let sni = ib["tls"]["server_name"].as_str().unwrap_or(s);
    let sni_param = if tls {
        format!("&sni={}", sni)
    } else {
        String::new()
    };
    let insec = if tls && insecure_flag(ib) {
        "&allowInsecure=1"
    } else {
        ""
    };
    Some(format!(
        "vless://{}@{}:{}?encryption=none&security={}&type=ws&path={}{}{}#{}",
        uuid,
        s,
        p,
        security,
        urlencoding::encode(path),
        sni_param,
        insec,
        fragment(tag)
    ))
}

fn vmess_ws(ib: &Value, user: &Value, s: &str, p: u64, tag: &str) -> Option<String> {
    let uuid = user["uuid"].as_str()?;
    let path = ib["transport"]["path"].as_str().unwrap_or("/");
    let sni = ib["tls"]["server_name"].as_str().unwrap_or(s);
    let tls = ib["tls"]["enabled"].as_bool().unwrap_or(false);
    let obj = serde_json::json!({
        "v":"2","ps":tag,
        "add":s,"port":p.to_string(),"id":uuid,"aid":"0",
        "net":"ws","type":"none","host":sni,"path":path,
        "tls": if tls {"tls"} else {""},
    });
    Some(format!("vmess://{}", STANDARD.encode(obj.to_string())))
}

fn shadowsocks(ib: &Value, user: &Value, s: &str, p: u64, tag: &str) -> Option<String> {
    let pw = user["password"].as_str()?;
    let m = ib["method"].as_str().unwrap_or("2022-blake3-aes-128-gcm");
    Some(format!(
        "ss://{}@{}:{}#{}",
        STANDARD.encode(format!("{}:{}", m, pw)),
        s,
        p,
        fragment(tag)
    ))
}

fn trojan(ib: &Value, user: &Value, s: &str, p: u64, tag: &str) -> Option<String> {
    let pw = user["password"].as_str()?;
    let sni = ib["tls"]["server_name"].as_str().unwrap_or(s);
    let insec = if insecure_flag(ib) {
        "&allowInsecure=1"
    } else {
        ""
    };
    Some(format!(
        "trojan://{}@{}:{}?security=tls&sni={}&type=tcp{}#{}",
        urlencoding::encode(pw),
        s,
        p,
        sni,
        insec,
        fragment(tag)
    ))
}

fn hysteria2(ib: &Value, user: &Value, s: &str, p: u64, tag: &str) -> Option<String> {
    let pw = user["password"].as_str()?;
    let sni = ib["tls"]["server_name"].as_str().unwrap_or(s);
    let insec = if insecure_flag(ib) { "&insecure=1" } else { "" };
    Some(format!(
        "hysteria2://{}@{}:{}?sni={}{}#{}",
        urlencoding::encode(pw),
        s,
        p,
        sni,
        insec,
        fragment(tag)
    ))
}

fn tuic(ib: &Value, user: &Value, s: &str, p: u64, tag: &str) -> Option<String> {
    let uuid = user["uuid"].as_str()?;
    let pw = user["password"].as_str()?;
    let sni = ib["tls"]["server_name"].as_str().unwrap_or(s);
    let insec = if insecure_flag(ib) {
        "&allow_insecure=1"
    } else {
        ""
    };
    Some(format!(
        "tuic://{}:{}@{}:{}?congestion_control=bbr&alpn=h3&sni={}&udp_relay_mode=native{}#{}",
        uuid,
        pw,
        s,
        p,
        sni,
        insec,
        fragment(tag)
    ))
}

fn anytls(ib: &Value, user: &Value, s: &str, p: u64, tag: &str) -> Option<String> {
    let pw = user["password"].as_str()?;
    let sni = ib["tls"]["server_name"].as_str().unwrap_or(s);
    let insec = if insecure_flag(ib) {
        "&allowInsecure=1"
    } else {
        ""
    };
    Some(format!(
        "anytls://{}@{}:{}?sni={}{}#{}",
        urlencoding::encode(pw),
        s,
        p,
        sni,
        insec,
        fragment(tag)
    ))
}

#[cfg(test)]
mod tests {
    use super::{generate_clash_yaml, generate_links};
    use serde_json::json;

    #[test]
    fn vless_ws_without_tls_exports_security_none() {
        let cfg = json!({
            "inbounds": [{
                "type": "vless",
                "tag": "ws",
                "listen_port": 8080,
                "users": [{ "name": "alice", "uuid": "de909d94-1d92-4a2f-9da8-c5b52a52282c" }],
                "transport": { "type": "ws", "path": "/vless" }
            }]
        });

        let links = generate_links(&cfg, "alice", "1.2.3.4").unwrap();

        assert_eq!(links.len(), 1);
        assert!(links[0].link.contains("security=none"));
        assert!(!links[0].link.contains("allowInsecure=1"));
    }

    #[test]
    fn vless_ws_with_self_signed_tls_exports_allow_insecure() {
        let cfg = json!({
            "inbounds": [{
                "type": "vless",
                "tag": "ws-tls",
                "listen_port": 443,
                "users": [{ "name": "alice", "uuid": "de909d94-1d92-4a2f-9da8-c5b52a52282c" }],
                "transport": { "type": "ws", "path": "/vless" },
                "tls": {
                    "enabled": true,
                    "server_name": "example.com",
                    "certificate_path": "/etc/sing-box/certs/ws-tls.crt"
                }
            }]
        });

        let links = generate_links(&cfg, "alice", "1.2.3.4").unwrap();

        assert_eq!(links.len(), 1);
        assert!(links[0].link.contains("security=tls"));
        assert!(links[0].link.contains("allowInsecure=1"));
        assert!(links[0].link.contains("sni=example.com"));
    }

    #[test]
    fn clash_yaml_includes_anytls_proxy() {
        let cfg = json!({
            "inbounds": [{
                "type": "anytls",
                "tag": "anytls-1",
                "listen_port": 443,
                "users": [{ "name": "alice", "password": "secret" }],
                "tls": {
                    "enabled": true,
                    "server_name": "example.com",
                    "certificate_path": "/etc/sing-box/certs/anytls-1.crt",
                    "key_path": "/etc/sing-box/certs/anytls-1.key"
                }
            }]
        });

        let yaml = generate_clash_yaml(&cfg, "alice", "1.2.3.4").unwrap();

        assert!(yaml.contains("type: anytls"));
        assert!(yaml.contains("name: anytls-1"));
    }

    #[test]
    fn share_link_fragment_uses_tag_only() {
        let cfg = json!({
            "inbounds": [{
                "type": "trojan",
                "tag": "trojan-1",
                "listen_port": 443,
                "users": [{ "name": "alice", "password": "secret" }],
                "tls": {
                    "enabled": true,
                    "server_name": "example.com",
                    "certificate_path": "/etc/sing-box/certs/trojan-1.crt",
                    "key_path": "/etc/sing-box/certs/trojan-1.key"
                }
            }]
        });

        let links = generate_links(&cfg, "alice", "1.2.3.4").unwrap();

        assert_eq!(links.len(), 1);
        assert!(links[0].link.ends_with("#trojan-1"));
        assert!(!links[0].link.ends_with("#trojan-1-alice"));
    }
}
