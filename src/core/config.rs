use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;

use crate::model::{node::{AddNodeRequest, Protocol}, user::User};

const META_FILE: &str = "/etc/sing-box-manager/nodes.meta.json";
pub const CERTS_DIR: &str = "/etc/sing-box-manager/certs";

#[derive(Debug, Default, Serialize, Deserialize)]
struct NodesMeta {
    /// tag -> { public_key (reality base64), ss_password (base64 16B) }
    #[serde(default)]
    nodes: HashMap<String, NodeMeta>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct NodeMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ss_password: Option<String>,
    /// 端口复用：sing-box 监听改走 127.0.0.1，订阅 URL 的端口写死 443（需自己配 nginx stream SNI 分流）
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub port_reuse: bool,
}

fn load_meta_file() -> NodesMeta {
    std::fs::read_to_string(META_FILE).ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_meta_file(m: &NodesMeta) -> Result<()> {
    if let Some(p) = Path::new(META_FILE).parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::write(META_FILE, serde_json::to_string_pretty(m)?)?;
    Ok(())
}

pub fn get_node_meta(tag: &str) -> Option<NodeMeta> {
    load_meta_file().nodes.get(tag).cloned()
}

pub fn set_node_meta(tag: &str, meta: NodeMeta) -> Result<()> {
    let mut m = load_meta_file();
    m.nodes.insert(tag.to_string(), meta);
    save_meta_file(&m)
}

pub fn remove_node_meta(tag: &str) {
    let mut m = load_meta_file();
    if m.nodes.remove(tag).is_some() {
        let _ = save_meta_file(&m);
    }
    // 同时清除证书文件
    let _ = std::fs::remove_file(Path::new(CERTS_DIR).join(format!("{}.crt", tag)));
    let _ = std::fs::remove_file(Path::new(CERTS_DIR).join(format!("{}.key", tag)));
}

pub fn load(path: &str) -> Result<Value> {
    Ok(serde_json::from_str(&std::fs::read_to_string(path)
        .with_context(|| format!("读取 {} 失败", path))?)?)
}

pub fn save(path: &str, json: &Value) -> Result<()> {
    let tmp = format!("{}.tmp", path);
    if let Some(parent) = Path::new(path).parent() {
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
    let removed = inbounds.len() < before;
    if removed { remove_node_meta(tag); }
    removed
}

/// 编辑已有节点：只能改 port / server_name / path / port_reuse（不改协议或密钥，否则应删重建）
pub fn edit_node(
    cfg: &mut Value, tag: &str,
    new_port: Option<u16>,
    new_server_name: Option<String>,
    new_path: Option<String>,
    new_port_reuse: Option<bool>,
) -> Result<()> {
    let inbounds = cfg.get_mut("inbounds").and_then(|v| v.as_array_mut())
        .ok_or_else(|| anyhow::anyhow!("inbounds 不是数组"))?;
    let ib = inbounds.iter_mut()
        .find(|ib| ib.get("tag").and_then(Value::as_str) == Some(tag))
        .ok_or_else(|| anyhow::anyhow!("节点不存在: {}", tag))?;
    if let Some(p) = new_port { ib["listen_port"] = json!(p); }
    if let Some(sn) = new_server_name {
        if let Some(tls) = ib.get_mut("tls").and_then(|v| v.as_object_mut()) {
            // 只对已经有 server_name 的 inbound 更新（避免向 hy2 这类不该有 server_name 的协议里硬塞字段）
            if tls.contains_key("server_name") {
                tls.insert("server_name".into(), json!(&sn));
                if let Some(reality) = tls.get_mut("reality").and_then(|v| v.as_object_mut()) {
                    if let Some(hs) = reality.get_mut("handshake").and_then(|v| v.as_object_mut()) {
                        hs.insert("server".into(), json!(&sn));
                    }
                }
            }
        }
    }
    if let Some(p) = new_path {
        if let Some(transport) = ib.get_mut("transport").and_then(|v| v.as_object_mut()) {
            transport.insert("path".into(), json!(p));
        }
    }
    if let Some(reuse) = new_port_reuse {
        // listen 字段按端口复用开关改写：开启 = 127.0.0.1（仅回环，给 nginx stream 回源用）；关闭 = ::（全部接口）
        ib["listen"] = Value::String(if reuse { "127.0.0.1".into() } else { "::".into() });
        // 同步更新 meta
        let mut meta = get_node_meta(tag).unwrap_or_default();
        meta.port_reuse = reuse;
        let _ = set_node_meta(tag, meta);
    }
    Ok(())
}

/// 将 managed 用户同步到所有用户型 inbound 的 users 数组。
/// 安全边界：仅移除 name 命中 managed 集合的用户条目，不触碰未托管的默认/旧账号。
/// 授权：`user.can_use_node(tag)` 为 false 的组合会被排除。
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
            let tag = ib.get("tag").and_then(Value::as_str).unwrap_or("").to_string();
            let additions: Vec<Value> = enabled.iter()
                .filter(|u| u.can_use_node(&tag))
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

    // v2ray_api.stats.users 仍包含所有启用用户（用于统计，不影响授权）
    sync_v2ray_api_users(cfg, &enabled, grpc_addr);
    synced
}

/// 读取 config.json 中全部 inbound tag 列表
pub fn list_tags(cfg: &Value) -> Vec<String> {
    cfg.get("inbounds").and_then(Value::as_array).map(|arr| {
        arr.iter().filter_map(|ib| ib.get("tag").and_then(Value::as_str).map(String::from)).collect()
    }).unwrap_or_default()
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
        "vmess" => Some(json!({"name": user.name, "uuid": user.uuid, "alterId": 0})),
        // shadowsocks 2022 系列方法要求 password 为 base64(16B)；
        // 用户的 uuid 恰好是 16B，取 as_bytes() 编码即可。
        "shadowsocks" => {
            let pw = STANDARD.encode(parse_uuid_bytes(&user.uuid));
            Some(json!({"name": user.name, "password": pw}))
        }
        "trojan" | "hysteria2" | "anytls" => {
            Some(json!({"name": user.name, "password": user.password}))
        }
        "tuic" => Some(json!({"name": user.name, "uuid": user.uuid, "password": user.password})),
        _ => None,
    }
}

fn parse_uuid_bytes(s: &str) -> [u8; 16] {
    uuid::Uuid::parse_str(s).map(|u| *u.as_bytes()).unwrap_or([0u8; 16])
}

fn build_inbound(req: &AddNodeRequest) -> Result<(Value, AddNodeMeta)> {
    match req.protocol {
        Protocol::VlessReality => {
            let (private_key, public_key) = generate_reality_keypair()?;
            let short_id = random_short_id();
            let sni = req.server_name.clone().unwrap_or_else(|| "www.apple.com".into());
            let _ = set_node_meta(&req.tag, NodeMeta {
                public_key: Some(public_key.clone()),
                ss_password: None,
                port_reuse: false,
            });
            let inbound = json!({
                "type": "vless",
                "tag":  req.tag,
                "listen": "::",
                "listen_port": req.listen_port,
                "users": [],
                "tls": {
                    "enabled": true,
                    "server_name": sni,
                    "reality": {
                        "enabled": true,
                        // handshake.server 跟 sni 一致，比硬编 www.apple.com 更合理
                        "handshake": { "server": sni, "server_port": 443 },
                        "private_key": private_key,
                        "short_id": [short_id.clone()]
                    }
                }
            });
            Ok((inbound, AddNodeMeta::RealityKeys { public_key, short_id }))
        }
        // vless-ws / vmess-ws 默认**不启用 TLS**：正常部署会在前面挂 nginx/caddy 做 TLS 终结，
        // 后端 ws 走明文；若要后端直连 TLS，可事后手工加 tls 块
        Protocol::VlessWs => {
            let path = req.path.clone().unwrap_or_else(|| "/vless".into());
            Ok((json!({
                "type": "vless",
                "tag":  req.tag,
                "listen": "::",
                "listen_port": req.listen_port,
                "users": [],
                "transport": {
                    "type": "ws", "path": path,
                    "max_early_data": 2048,
                    "early_data_header_name": "Sec-WebSocket-Protocol"
                }
            }), AddNodeMeta::Plain))
        }
        Protocol::VmessWs => {
            let path = req.path.clone().unwrap_or_else(|| "/vmess".into());
            Ok((json!({
                "type": "vmess",
                "tag":  req.tag,
                "listen": "::",
                "listen_port": req.listen_port,
                "users": [],
                "transport": {
                    "type": "ws", "path": path,
                    "max_early_data": 2048,
                    "early_data_header_name": "Sec-WebSocket-Protocol"
                }
            }), AddNodeMeta::Plain))
        }
        Protocol::Trojan => {
            let sni = req.server_name.clone().unwrap_or_else(|| "bing.com".into());
            let (crt, key) = ensure_self_signed_cert(&req.tag, &sni)?;
            Ok((json!({
                "type": "trojan",
                "tag":  req.tag,
                "listen": "::",
                "listen_port": req.listen_port,
                "users": [],
                "tls": {
                    "enabled": true,
                    "server_name": sni,
                    "certificate_path": crt,
                    "key_path": key
                }
            }), AddNodeMeta::Plain))
        }
        Protocol::Shadowsocks => {
            let method = "2022-blake3-aes-128-gcm";
            let ss_pwd = random_b64_16();
            let _ = set_node_meta(&req.tag, NodeMeta {
                public_key: None, ss_password: Some(ss_pwd.clone()),
                port_reuse: false,
            });
            Ok((json!({
                "type": "shadowsocks",
                "tag":  req.tag,
                "listen": "::",
                "listen_port": req.listen_port,
                "method": method,
                "password": ss_pwd,
                "users": []
            }), AddNodeMeta::Plain))
        }
        Protocol::Hysteria2 => {
            // hy2 inbound 不需要 server_name（sing-box 官方示例亦无此字段）；
            // 证书 CN 用 tag 本身，server_name 交由客户端从 URL 的 sni 决定（默认回落到 server）。
            let (crt, key) = ensure_self_signed_cert(&req.tag, &req.tag)?;
            Ok((json!({
                "type": "hysteria2",
                "tag":  req.tag,
                "listen": "::",
                "listen_port": req.listen_port,
                "users": [],
                "tls": {
                    "enabled": true,
                    "alpn": ["h3"],
                    "certificate_path": crt,
                    "key_path": key
                }
            }), AddNodeMeta::Plain))
        }
        Protocol::Tuic => {
            let sni = req.server_name.clone().unwrap_or_else(|| "bing.com".into());
            let (crt, key) = ensure_self_signed_cert(&req.tag, &sni)?;
            Ok((json!({
                "type": "tuic",
                "tag":  req.tag,
                "listen": "::",
                "listen_port": req.listen_port,
                "users": [],
                "congestion_control": "bbr",
                "tls": {
                    "enabled": true,
                    "alpn": ["h3"],
                    "server_name": sni,
                    "certificate_path": crt,
                    "key_path": key
                }
            }), AddNodeMeta::Plain))
        }
        Protocol::Anytls => {
            let sni = req.server_name.clone().unwrap_or_else(|| "bing.com".into());
            let (crt, key) = ensure_self_signed_cert(&req.tag, &sni)?;
            Ok((json!({
                "type": "anytls",
                "tag":  req.tag,
                "listen": "::",
                "listen_port": req.listen_port,
                "users": [],
                "padding_scheme": [],
                "tls": {
                    "enabled": true,
                    "alpn": ["h2", "http/1.1"],
                    "server_name": sni,
                    "certificate_path": crt,
                    "key_path": key
                }
            }), AddNodeMeta::Plain))
        }
        Protocol::Unknown => Ok((json!({
            "type": "direct",
            "tag":  req.tag,
            "listen": "::",
            "listen_port": req.listen_port
        }), AddNodeMeta::Plain)),
    }
}

/// 为 TLS 协议按需生成自签 cert/key 文件。使用 EC P-256（比 RSA 小很多，握手快）。
fn ensure_self_signed_cert(tag: &str, sni: &str) -> Result<(String, String)> {
    let base = Path::new(CERTS_DIR);
    std::fs::create_dir_all(base).with_context(|| format!("创建证书目录 {} 失败", base.display()))?;
    let crt = base.join(format!("{}.crt", tag));
    let key = base.join(format!("{}.key", tag));
    if crt.exists() && key.exists() { return Ok((crt.display().to_string(), key.display().to_string())); }

    // 1. 生成 EC P-256 私钥
    let status = Command::new("openssl")
        .args(["ecparam", "-name", "prime256v1", "-genkey", "-noout", "-out"])
        .arg(&key).status()
        .with_context(|| "调用 openssl ecparam 失败（请确保已安装 openssl）")?;
    if !status.success() { anyhow::bail!("openssl 生成 EC 私钥失败 (tag={})", tag); }

    // 2. 用该私钥签一个 100 年有效的自签证书
    let status = Command::new("openssl")
        .args(["req", "-x509", "-new", "-key"])
        .arg(&key)
        .arg("-out").arg(&crt)
        .args(["-days", "36500", "-nodes", "-subj"])
        .arg(format!("/CN={}", sni))
        .status()
        .with_context(|| "调用 openssl req 失败")?;
    if !status.success() { anyhow::bail!("openssl 生成自签证书失败 (tag={})", tag); }
    Ok((crt.display().to_string(), key.display().to_string()))
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

/// 生成 base64(16 随机字节)，用于 shadowsocks 2022 系列方法的密钥 / 密码
fn random_b64_16() -> String {
    STANDARD.encode(uuid::Uuid::new_v4().as_bytes())
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
