use anyhow::{Context, Result};
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::process::Command;

use crate::model::{node::{AddNodeRequest, Protocol}, user::User};

pub fn load(path: &str) -> Result<Value> {
    Ok(serde_json::from_str(&std::fs::read_to_string(path)
        .with_context(|| format!("读取 {} 失败", path))?)?)
}

pub fn save(path: &str, json: &Value) -> Result<()> {
    let tmp = format!("{}.tmp", path);
    if let Some(parent) = std::path::Path::new(path).parent() {
        if !parent.as_os_str().is_empty() { std::fs::create_dir_all(parent).ok(); }
    }
    std::fs::write(&tmp, serde_json::to_string_pretty(json)?)?;
    // Unix: rename 覆盖已有文件；Windows 下需先删除（项目定位 Linux，不处理）。
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[derive(Debug, Clone)]
pub enum AddNodeMeta {
    Plain,
    /// 新建 vless-reality 节点时自动生成的密钥信息，用于回显给用户
    RealityKeys { public_key: String, short_id: String },
}

pub fn add_node(cfg: &mut Value, req: &AddNodeRequest) -> Result<AddNodeMeta> {
    let root = ensure_object(cfg);
    let inbounds = root.entry("inbounds").or_insert_with(|| Value::Array(vec![]));
    let inbounds = inbounds.as_array_mut()
        .ok_or_else(|| anyhow::anyhow!("inbounds 字段不是数组"))?;
    if inbounds.iter().any(|ib| ib["tag"].as_str() == Some(&req.tag)) {
        anyhow::bail!("节点 tag 已存在: {}", req.tag);
    }
    let (inbound, meta) = build_inbound(req)?;
    inbounds.push(inbound);
    Ok(meta)
}

/// 按 tag 移除 inbound。返回是否确实移除了节点。
pub fn remove_node(cfg: &mut Value, tag: &str) -> bool {
    let Some(inbounds) = cfg.get_mut("inbounds").and_then(|v| v.as_array_mut()) else { return false; };
    let before = inbounds.len();
    inbounds.retain(|ib| ib.get("tag").and_then(Value::as_str) != Some(tag));
    inbounds.len() < before
}

/// 将 managed 用户同步到所有用户型 inbound 的 users 数组。
/// 安全边界：仅移除 name 命中 managed 集合的用户条目，不触碰未托管的默认/旧账号。
pub fn sync_users(cfg: &mut Value, users: &[User], grpc_addr: &str) -> usize {
    let managed: HashSet<&str> = users.iter().map(|u| u.name.as_str()).collect();
    let enabled: Vec<&User> = users.iter()
        .filter(|u| u.enabled && !u.is_expired() && !u.is_over_quota())
        .collect();

    let mut synced = 0;
    if let Some(inbounds) = cfg.get_mut("inbounds").and_then(|v| v.as_array_mut()) {
        for ib in inbounds {
            let typ = ib.get("type").and_then(Value::as_str).unwrap_or("").to_string();
            if !matches!(typ.as_str(),
                "vless" | "vmess" | "trojan" | "shadowsocks" | "hysteria2" | "tuic" | "anytls")
            {
                continue;
            }
            let additions: Vec<Value> = enabled.iter()
                .filter_map(|user| build_user_value(ib, user))
                .collect();
            let arr = ib.as_object_mut()
                .and_then(|o| o.entry("users").or_insert_with(|| Value::Array(vec![])).as_array_mut());
            let Some(arr) = arr else { continue };
            arr.retain(|item| {
                match item.get("name").and_then(Value::as_str) {
                    Some(n) => !managed.contains(n),
                    None => true,
                }
            });
            for value in additions {
                arr.push(value);
                synced += 1;
            }
        }
    }

    sync_v2ray_api_users(cfg, &enabled, grpc_addr);
    synced
}

fn build_user_value(ib: &Value, user: &User) -> Option<Value> {
    let typ = ib.get("type").and_then(Value::as_str).unwrap_or("");
    match typ {
        "vless" => {
            let mut value = json!({"name": user.name, "uuid": user.uuid});
            if ib["tls"]["reality"]["enabled"].as_bool() == Some(true) {
                value["flow"] = Value::String("xtls-rprx-vision".into());
            }
            Some(value)
        }
        "vmess" => Some(json!({"name": user.name, "uuid": user.uuid})),
        "trojan" | "hysteria2" | "anytls" | "shadowsocks" => {
            Some(json!({"name": user.name, "password": user.password}))
        }
        "tuic" => Some(json!({"name": user.name, "uuid": user.uuid, "password": user.password})),
        _ => None,
    }
}

fn build_inbound(req: &AddNodeRequest) -> Result<(Value, AddNodeMeta)> {
    match req.protocol {
        Protocol::VlessReality => {
            let (private_key, public_key) = generate_reality_keypair()?;
            let short_id = random_short_id();
            let inbound = json!({
                "tag": req.tag, "type": "vless", "listen": "::",
                "listen_port": req.listen_port, "users": [],
                "tls": {
                    "enabled": true,
                    "server_name": req.server_name.clone().unwrap_or_else(|| "www.apple.com".into()),
                    "reality": {
                        "enabled": true,
                        "private_key": private_key,
                        // 非标准字段，sing-box 会忽略；供订阅生成读 public_key 用
                        "public_key": public_key.clone(),
                        "short_id": [short_id.clone()],
                        "handshake": {
                            "server": req.server_name.clone().unwrap_or_else(|| "www.apple.com".into()),
                            "server_port": 443
                        }
                    }
                }
            });
            Ok((inbound, AddNodeMeta::RealityKeys { public_key, short_id }))
        }
        Protocol::VlessWs => Ok((json!({
            "tag": req.tag, "type": "vless", "listen": "::",
            "listen_port": req.listen_port, "users": [],
            "transport": { "type": "ws", "path": req.path.clone().unwrap_or_else(|| "/vless".into()) },
            "tls": { "enabled": true, "server_name": req.server_name.clone().unwrap_or_default() }
        }), AddNodeMeta::Plain)),
        Protocol::VmessWs => Ok((json!({
            "tag": req.tag, "type": "vmess", "listen": "::",
            "listen_port": req.listen_port, "users": [],
            "transport": { "type": "ws", "path": req.path.clone().unwrap_or_else(|| "/vmess".into()) },
            "tls": { "enabled": true, "server_name": req.server_name.clone().unwrap_or_default() }
        }), AddNodeMeta::Plain)),
        Protocol::Trojan => Ok((json!({
            "tag": req.tag, "type": "trojan", "listen": "::",
            "listen_port": req.listen_port, "users": [],
            "tls": { "enabled": true, "server_name": req.server_name.clone().unwrap_or_default() }
        }), AddNodeMeta::Plain)),
        Protocol::Shadowsocks => Ok((json!({
            "tag": req.tag, "type": "shadowsocks", "listen": "::",
            "listen_port": req.listen_port, "users": [],
            "method": "2022-blake3-aes-128-gcm"
        }), AddNodeMeta::Plain)),
        Protocol::Hysteria2 => Ok((json!({
            "tag": req.tag, "type": "hysteria2", "listen": "::",
            "listen_port": req.listen_port, "users": [],
            "tls": { "enabled": true, "server_name": req.server_name.clone().unwrap_or_else(|| "bing.com".into()) }
        }), AddNodeMeta::Plain)),
        Protocol::Tuic => Ok((json!({
            "tag": req.tag, "type": "tuic", "listen": "::",
            "listen_port": req.listen_port, "users": [],
            "tls": { "enabled": true, "server_name": req.server_name.clone().unwrap_or_else(|| "bing.com".into()) }
        }), AddNodeMeta::Plain)),
        Protocol::Anytls => Ok((json!({
            "tag": req.tag, "type": "anytls", "listen": "::",
            "listen_port": req.listen_port, "users": [],
            "tls": { "enabled": true, "server_name": req.server_name.clone().unwrap_or_else(|| "bing.com".into()) }
        }), AddNodeMeta::Plain)),
        Protocol::Unknown => Ok((json!({
            "tag": req.tag, "type": "unknown", "listen": "::",
            "listen_port": req.listen_port, "users": []
        }), AddNodeMeta::Plain)),
    }
}

/// 调用 `sing-box generate reality-keypair`，返回 (private_key, public_key)
fn generate_reality_keypair() -> Result<(String, String)> {
    let bin = ["/usr/local/bin/sing-box", "/usr/bin/sing-box"]
        .iter().find(|p| std::path::Path::new(p).exists())
        .copied()
        .unwrap_or("sing-box");
    let out = Command::new(bin).args(["generate", "reality-keypair"]).output()
        .with_context(|| "调用 sing-box generate reality-keypair 失败")?;
    if !out.status.success() {
        anyhow::bail!("sing-box generate reality-keypair 返回非零: {}", String::from_utf8_lossy(&out.stderr));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut priv_k = None;
    let mut pub_k  = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("PrivateKey:").or_else(|| line.strip_prefix("PrivateKey =")) {
            priv_k = Some(v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("PublicKey:").or_else(|| line.strip_prefix("PublicKey =")) {
            pub_k = Some(v.trim().to_string());
        }
    }
    match (priv_k, pub_k) {
        (Some(a), Some(b)) => Ok((a, b)),
        _ => anyhow::bail!("解析 reality-keypair 输出失败: {}", text),
    }
}

fn random_short_id() -> String {
    // 8 hex 字符 = 4 字节。用 UUIDv4 前 8 位足够随机
    uuid::Uuid::new_v4().simple().to_string().chars().take(8).collect()
}

fn sync_v2ray_api_users(cfg: &mut Value, users: &[&User], grpc_addr: &str) {
    let root = ensure_object(cfg);
    let experimental = root.entry("experimental").or_insert_with(|| json!({}));
    let experimental = ensure_object(experimental);
    let api = experimental.entry("v2ray_api").or_insert_with(|| json!({}));
    let api = ensure_object(api);
    api.entry("listen").or_insert_with(|| Value::String(grpc_addr.to_string()));
    let stats = api.entry("stats").or_insert_with(|| json!({}));
    let stats = ensure_object(stats);
    stats.insert("enabled".into(), Value::Bool(true));
    stats.insert(
        "users".into(),
        Value::Array(users.iter().map(|u| Value::String(u.name.clone())).collect()),
    );
}

fn ensure_object(value: &mut Value) -> &mut Map<String, Value> {
    if !value.is_object() {
        *value = json!({});
    }
    value.as_object_mut().expect("value should be object")
}
