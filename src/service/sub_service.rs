use anyhow::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct ShareLink { pub protocol: String, pub link: String }

pub fn generate_links(cfg: &Value, username: &str, server: &str) -> Result<Vec<ShareLink>> {
    let mut links = Vec::new();
    let Some(inbounds) = cfg["inbounds"].as_array() else { return Ok(links); };
    for ib in inbounds {
        let tag  = ib["tag"].as_str().unwrap_or("");
        let typ  = ib["type"].as_str().unwrap_or("");
        let port = ib["listen_port"].as_u64().unwrap_or(0);
        let Some(user) = find_user(ib, username) else { continue; };
        let link = match typ {
            "vless" => {
                if ib["tls"]["reality"]["enabled"].as_bool() == Some(true) {
                    vless_reality(ib, user, server, port, tag)
                } else if ib["transport"]["type"].as_str() == Some("ws") {
                    vless_ws(ib, user, server, port, tag)
                } else { None }
            }
            "vmess"       => vmess_ws(ib, user, server, port, tag),
            "shadowsocks" => shadowsocks(ib, user, server, port, tag),
            "trojan"      => trojan(ib, user, server, port, tag),
            "hysteria2"   => hysteria2(ib, user, server, port, tag, username),
            "tuic"        => tuic(ib, user, server, port, tag, username),
            "anytls"      => anytls(ib, user, server, port, tag, username),
            _ => None,
        };
        if let Some(l) = link {
            links.push(ShareLink { protocol: typ.into(), link: l });
        }
    }
    Ok(links)
}

pub fn generate_subscription(links: &[ShareLink]) -> String {
    STANDARD.encode(links.iter().map(|l| l.link.as_str()).collect::<Vec<_>>().join("\n"))
}

fn find_user<'a>(ib: &'a Value, name: &str) -> Option<&'a Value> {
    ib["users"].as_array()?.iter().find(|u| u["name"].as_str() == Some(name))
}

/// 是否在订阅中加 allowInsecure=1：自签名 / 无证书路径时才加。
fn insecure_flag(ib: &Value) -> bool {
    let tls = &ib["tls"];
    if tls["reality"]["enabled"].as_bool() == Some(true) { return false; }
    tls["certificate_path"].as_str().is_none()
        && tls["acme"].as_object().is_none()
}

fn fragment(tag: &str, name: &str) -> String {
    urlencoding::encode(&format!("{}-{}", tag, name)).into_owned()
}

fn vless_reality(ib: &Value, user: &Value, s: &str, p: u64, tag: &str) -> Option<String> {
    let uuid = user["uuid"].as_str()?;
    let name = user["name"].as_str()?;
    let sni = ib["tls"]["server_name"].as_str().unwrap_or("www.apple.com");
    let pk  = ib["tls"]["reality"]["public_key"].as_str().unwrap_or("");
    let sid = ib["tls"]["reality"]["short_id"].as_array()
        .and_then(|a| a.first()).and_then(Value::as_str).unwrap_or("");
    Some(format!(
        "vless://{}@{}:{}?encryption=none&flow=xtls-rprx-vision&security=reality&sni={}&fp=chrome&pbk={}&sid={}&type=tcp#{}",
        uuid, s, p, sni, pk, sid, fragment(tag, name)))
}

fn vless_ws(ib: &Value, user: &Value, s: &str, p: u64, tag: &str) -> Option<String> {
    let uuid = user["uuid"].as_str()?;
    let name = user["name"].as_str()?;
    let path = ib["transport"]["path"].as_str().unwrap_or("/");
    let sni  = ib["tls"]["server_name"].as_str().unwrap_or(s);
    let insec = if insecure_flag(ib) { "&allowInsecure=1" } else { "" };
    Some(format!(
        "vless://{}@{}:{}?encryption=none&security=tls&sni={}&type=ws&path={}{}#{}",
        uuid, s, p, sni, urlencoding::encode(path), insec, fragment(tag, name)))
}

fn vmess_ws(ib: &Value, user: &Value, s: &str, p: u64, tag: &str) -> Option<String> {
    let uuid = user["uuid"].as_str()?;
    let name = user["name"].as_str()?;
    let path = ib["transport"]["path"].as_str().unwrap_or("/");
    let sni  = ib["tls"]["server_name"].as_str().unwrap_or(s);
    let tls  = ib["tls"]["enabled"].as_bool().unwrap_or(false);
    let obj = serde_json::json!({
        "v":"2","ps":format!("{}-{}", tag, name),
        "add":s,"port":p.to_string(),"id":uuid,"aid":"0",
        "net":"ws","type":"none","host":sni,"path":path,
        "tls": if tls {"tls"} else {""},
    });
    Some(format!("vmess://{}", STANDARD.encode(obj.to_string())))
}

fn shadowsocks(ib: &Value, user: &Value, s: &str, p: u64, tag: &str) -> Option<String> {
    let pw   = user["password"].as_str()?;
    let name = user["name"].as_str()?;
    let m = ib["method"].as_str().unwrap_or("2022-blake3-aes-128-gcm");
    Some(format!("ss://{}@{}:{}#{}",
        STANDARD.encode(format!("{}:{}", m, pw)), s, p, fragment(tag, name)))
}

fn trojan(ib: &Value, user: &Value, s: &str, p: u64, tag: &str) -> Option<String> {
    let pw   = user["password"].as_str()?;
    let name = user["name"].as_str()?;
    let sni = ib["tls"]["server_name"].as_str().unwrap_or(s);
    let insec = if insecure_flag(ib) { "&allowInsecure=1" } else { "" };
    Some(format!(
        "trojan://{}@{}:{}?security=tls&sni={}&type=tcp{}#{}",
        urlencoding::encode(pw), s, p, sni, insec, fragment(tag, name)))
}

fn hysteria2(ib: &Value, user: &Value, s: &str, p: u64, tag: &str, name: &str) -> Option<String> {
    let pw = user["password"].as_str()?;
    let sni = ib["tls"]["server_name"].as_str().unwrap_or(s);
    let insec = if insecure_flag(ib) { "&insecure=1" } else { "" };
    Some(format!(
        "hysteria2://{}@{}:{}?sni={}{}#{}",
        urlencoding::encode(pw), s, p, sni, insec, fragment(tag, name)))
}

fn tuic(ib: &Value, user: &Value, s: &str, p: u64, tag: &str, name: &str) -> Option<String> {
    let uuid = user["uuid"].as_str()?;
    let pw   = user["password"].as_str()?;
    let sni  = ib["tls"]["server_name"].as_str().unwrap_or(s);
    let insec = if insecure_flag(ib) { "&allow_insecure=1" } else { "" };
    Some(format!(
        "tuic://{}:{}@{}:{}?congestion_control=bbr&alpn=h3&sni={}&udp_relay_mode=native{}#{}",
        uuid, pw, s, p, sni, insec, fragment(tag, name)))
}

fn anytls(ib: &Value, user: &Value, s: &str, p: u64, tag: &str, name: &str) -> Option<String> {
    let pw = user["password"].as_str()?;
    let sni = ib["tls"]["server_name"].as_str().unwrap_or(s);
    let insec = if insecure_flag(ib) { "&allowInsecure=1" } else { "" };
    Some(format!(
        "anytls://{}@{}:{}?sni={}{}#{}",
        urlencoding::encode(pw), s, p, sni, insec, fragment(tag, name)))
}
