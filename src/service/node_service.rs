use std::time::Duration;

use serde_json::Value;
use crate::model::node::{InboundNode, Protocol};

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
        "vmess"       => Protocol::VmessWs,
        "shadowsocks" => Protocol::Shadowsocks,
        "trojan"      => Protocol::Trojan,
        "tuic"        => Protocol::Tuic,
        "anytls"      => Protocol::Anytls,
        "hysteria2"   => Protocol::Hysteria2,
        _             => Protocol::Unknown,
    }
}

/// 查询本机公网 IP，带超时与 fallback。
pub async fn get_server_ip() -> String {
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .connect_timeout(Duration::from_secs(2))
        .build() else { return "127.0.0.1".into(); };
    for url in &["https://api4.ipify.org", "https://ifconfig.me/ip"] {
        if let Ok(resp) = client.get(*url).send().await {
            if let Ok(text) = resp.text().await {
                let t = text.trim();
                if !t.is_empty() && t.len() <= 64 { return t.to_string(); }
            }
        }
    }
    "127.0.0.1".into()
}
