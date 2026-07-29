use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use crate::model::{
    node::{AddNodeRequest, EditNodeRequest, Protocol},
    user::User,
};

const META_FILE: &str = "/etc/sing-box/manager/nodes.meta.json";
pub const CERTS_DIR: &str = "/etc/sing-box/certs";

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
    /// 订阅导出时优先使用 IPv6 地址
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ipv6: bool,
    /// 中转机地址（IP 或域名）。设了之后订阅导出的 server 用它，替代本机公网地址。
    /// 只影响订阅链接，sing-box inbound 不变——转发由中转机自己实现。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay_host: Option<String>,
    /// 中转机端口。留空则沿用节点自身对外端口。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay_port: Option<u16>,
}

impl NodeMeta {
    /// 给 UI 展示用的中转摘要，未启用时返回 None。
    pub fn relay_label(&self) -> Option<String> {
        let host = self.relay_host.as_deref()?.trim();
        if host.is_empty() {
            return None;
        }
        Some(match self.relay_port {
            Some(p) => format!("{}:{}", host, p),
            None => host.to_string(),
        })
    }
}

fn load_meta_file() -> NodesMeta {
    std::fs::read_to_string(META_FILE)
        .ok()
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

/// 配置突变期间需要伴随写出的 NodeMeta 副作用。由 add_node / edit_node /
/// remove_node 收集，由 mutate_config_locked 在 lock commit 成功后统一 apply，
/// 保证 config.json 与 nodes.meta.json 不会出现"一边落盘、一边没落盘"的不一致。
#[derive(Debug, Clone)]
pub enum MetaOp {
    Set(String, NodeMeta),
    Remove(String),
}

pub fn apply_meta_ops(ops: &[MetaOp]) {
    for op in ops {
        match op {
            MetaOp::Set(tag, meta) => {
                if let Err(e) = set_node_meta(tag, meta.clone()) {
                    tracing::warn!(tag = %tag, error = %e, "写 NodeMeta 失败");
                }
            }
            MetaOp::Remove(tag) => remove_node_meta(tag),
        }
    }
}

/// 把 ops 里同 tag 的 Set 合并；没有就基于现有 meta 新建一个再 push。
fn merge_or_push_meta(ops: &mut Vec<MetaOp>, tag: &str, mutate: impl FnOnce(&mut NodeMeta)) {
    for op in ops.iter_mut() {
        if let MetaOp::Set(t, m) = op {
            if t == tag {
                mutate(m);
                return;
            }
        }
    }
    let mut m = get_node_meta(tag).unwrap_or_default();
    mutate(&mut m);
    ops.push(MetaOp::Set(tag.to_string(), m));
}

pub fn load(path: &str) -> Result<Value> {
    Ok(serde_json::from_str(
        &std::fs::read_to_string(path).with_context(|| format!("读取 {} 失败", path))?,
    )?)
}

#[derive(Debug, Clone)]
pub enum AddNodeMeta {
    Plain,
    /// 新建 vless-reality 节点时自动生成的密钥信息，用于回显给用户
    RealityKeys {
        public_key: String,
        short_id: String,
    },
}

/// 新增 inbound。`binary_path` 用于 reality 密钥生成时定位 sing-box 二进制，
/// 传 None 时回落到常见安装路径。
pub fn add_node(
    cfg: &mut Value,
    req: &AddNodeRequest,
    binary_path: Option<&str>,
    ops: &mut Vec<MetaOp>,
) -> Result<AddNodeMeta> {
    let root = ensure_object(cfg);
    let inbounds = root
        .entry("inbounds")
        .or_insert_with(|| Value::Array(vec![]));
    let inbounds = inbounds
        .as_array_mut()
        .ok_or_else(|| anyhow::anyhow!("inbounds 字段不是数组"))?;
    if inbounds
        .iter()
        .any(|ib| ib["tag"].as_str() == Some(&req.tag))
    {
        anyhow::bail!("节点 tag 已存在: {}", req.tag);
    }
    let (mut inbound, meta) = build_inbound(req, binary_path, ops)?;
    if req.port_reuse {
        // 端口复用：inbound 只监听 127.0.0.1，由 nginx stream 做 SNI 分流回源
        inbound["listen"] = json!("127.0.0.1");
    }
    // 导出侧的 meta 统一在这里落盘。
    // 以前 ipv6 只写在 build_inbound 的 reality / shadowsocks 两个分支里，
    // 其余 6 种协议加 --ipv6 会被静默丢掉；集中到这里顺带修掉。
    merge_or_push_meta(ops, &req.tag, |m| {
        m.port_reuse = req.port_reuse;
        m.ipv6 = req.ipv6;
        if req.relay.is_enabled() {
            m.relay_host = Some(req.relay.host.trim().to_string());
            m.relay_port = req.relay.port;
        } else {
            m.relay_host = None;
            m.relay_port = None;
        }
    });
    inbounds.push(inbound);
    Ok(meta)
}

/// 按 tag 移除 inbound。返回是否确实移除了节点。
pub fn remove_node(cfg: &mut Value, tag: &str, ops: &mut Vec<MetaOp>) -> bool {
    let Some(inbounds) = cfg.get_mut("inbounds").and_then(|v| v.as_array_mut()) else {
        return false;
    };
    let before = inbounds.len();
    inbounds.retain(|ib| ib.get("tag").and_then(Value::as_str) != Some(tag));
    let removed = inbounds.len() < before;
    if removed {
        ops.push(MetaOp::Remove(tag.to_string()));
    }
    removed
}

/// 编辑已有节点：只能改 port / server_name / path / port_reuse / ipv6 / 中转
/// （不改协议或密钥，否则应删重建）
pub fn edit_node(cfg: &mut Value, req: &EditNodeRequest, ops: &mut Vec<MetaOp>) -> Result<()> {
    let tag = req.tag.as_str();
    let inbounds = cfg
        .get_mut("inbounds")
        .and_then(|v| v.as_array_mut())
        .ok_or_else(|| anyhow::anyhow!("inbounds 不是数组"))?;
    let ib = inbounds
        .iter_mut()
        .find(|ib| ib.get("tag").and_then(Value::as_str) == Some(tag))
        .ok_or_else(|| anyhow::anyhow!("节点不存在: {}", tag))?;
    if let Some(p) = req.listen_port {
        ib["listen_port"] = json!(p);
    }
    if let Some(sn) = req.server_name.as_deref() {
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
    if let Some(p) = req.path.as_deref() {
        if let Some(transport) = ib.get_mut("transport").and_then(|v| v.as_object_mut()) {
            transport.insert("path".into(), json!(p));
        }
    }
    if let Some(reuse) = req.port_reuse {
        // listen 字段按端口复用开关改写：开启 = 127.0.0.1（仅回环，给 nginx stream 回源用）；关闭 = ::（全部接口）
        ib["listen"] = Value::String(if reuse {
            "127.0.0.1".into()
        } else {
            "::".into()
        });
        merge_or_push_meta(ops, tag, |m| m.port_reuse = reuse);
    }
    if let Some(ipv6) = req.ipv6 {
        merge_or_push_meta(ops, tag, |m| m.ipv6 = ipv6);
    }
    if let Some(relay) = req.relay.as_ref() {
        // 整组覆盖：地址为空即关闭中转，顺带把端口也清掉，
        // 避免留下"没有地址却有端口"的半截状态。
        merge_or_push_meta(ops, tag, |m| {
            if relay.is_enabled() {
                m.relay_host = Some(relay.host.trim().to_string());
                m.relay_port = relay.port;
            } else {
                m.relay_host = None;
                m.relay_port = None;
            }
        });
    }
    Ok(())
}

/// 将数据库用户重建到所有用户型 inbound 的 users 数组。
/// 安全边界：仅保留协议默认占位账号和无 name 的手工条目，其余命名用户条目由 manager 全量重建。
/// 授权：`user.can_use_node(tag)` 为 false 的组合会被排除。
pub fn sync_users(cfg: &mut Value, users: &[User], grpc_addr: &str) -> usize {
    let enabled: Vec<&User> = users
        .iter()
        .filter(|u| u.enabled && !u.is_expired() && !u.is_over_quota())
        .collect();

    let mut synced = 0;
    if let Some(inbounds) = cfg.get_mut("inbounds").and_then(|v| v.as_array_mut()) {
        for ib in inbounds {
            let typ = ib
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if !matches!(
                typ.as_str(),
                "vless" | "vmess" | "trojan" | "shadowsocks" | "hysteria2" | "tuic" | "anytls"
            ) {
                continue;
            }
            let tag = ib
                .get("tag")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let default_name = default_user_name_for_type(&typ);
            let additions: Vec<Value> = enabled
                .iter()
                .filter(|u| u.can_use_node(&tag))
                .filter_map(|user| build_user_value(ib, user))
                .collect();
            let arr = ib.as_object_mut().and_then(|o| {
                o.entry("users")
                    .or_insert_with(|| Value::Array(vec![]))
                    .as_array_mut()
            });
            let Some(arr) = arr else { continue };
            arr.retain(|item| should_preserve_user_entry(item, default_name));
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

fn default_user_name_for_type(typ: &str) -> &'static str {
    match typ {
        "hysteria2" => "hy2-default",
        "tuic" => "tuic-default",
        "anytls" => "anytls-default",
        _ => "default",
    }
}

fn should_preserve_user_entry(item: &Value, default_name: &str) -> bool {
    match item.get("name").and_then(Value::as_str) {
        Some(name) => name == default_name,
        None => true,
    }
}

/// 读取 config.json 中全部 inbound tag 列表
pub fn list_tags(cfg: &Value) -> Vec<String> {
    cfg.get("inbounds")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|ib| ib.get("tag").and_then(Value::as_str).map(String::from))
                .collect()
        })
        .unwrap_or_default()
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
    uuid::Uuid::parse_str(s)
        .map(|u| *u.as_bytes())
        .unwrap_or([0u8; 16])
}

fn build_inbound(
    req: &AddNodeRequest,
    binary_path: Option<&str>,
    ops: &mut Vec<MetaOp>,
) -> Result<(Value, AddNodeMeta)> {
    match req.protocol {
        Protocol::VlessReality => {
            let (private_key, public_key) = generate_reality_keypair(binary_path)?;
            let short_id = random_short_id();
            let sni = req
                .server_name
                .clone()
                .unwrap_or_else(|| "www.apple.com".into());
            ops.push(MetaOp::Set(
                req.tag.clone(),
                NodeMeta {
                    public_key: Some(public_key.clone()),
                    ..Default::default()
                },
            ));
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
            Ok((
                inbound,
                AddNodeMeta::RealityKeys {
                    public_key,
                    short_id,
                },
            ))
        }
        // vless-ws / vmess-ws 默认**不启用 TLS**：正常部署会在前面挂 nginx/caddy 做 TLS 终结，
        // 后端 ws 走明文；若要后端直连 TLS，可事后手工加 tls 块
        Protocol::VlessWs => {
            let path = req.path.clone().unwrap_or_else(|| "/vless".into());
            Ok((
                json!({
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
                }),
                AddNodeMeta::Plain,
            ))
        }
        Protocol::VmessWs => {
            let path = req.path.clone().unwrap_or_else(|| "/vmess".into());
            Ok((
                json!({
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
                }),
                AddNodeMeta::Plain,
            ))
        }
        Protocol::Trojan => {
            let sni = req.server_name.clone().unwrap_or_else(|| "bing.com".into());
            let (crt, key) = ensure_self_signed_cert(&req.tag, &sni)?;
            Ok((
                json!({
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
                }),
                AddNodeMeta::Plain,
            ))
        }
        Protocol::Shadowsocks => {
            let method = "2022-blake3-aes-128-gcm";
            let ss_pwd = random_b64_16();
            ops.push(MetaOp::Set(
                req.tag.clone(),
                NodeMeta {
                    ss_password: Some(ss_pwd.clone()),
                    ..Default::default()
                },
            ));
            Ok((
                json!({
                    "type": "shadowsocks",
                    "tag":  req.tag,
                    "listen": "::",
                    "listen_port": req.listen_port,
                    "method": method,
                    "password": ss_pwd,
                    "users": []
                }),
                AddNodeMeta::Plain,
            ))
        }
        Protocol::Hysteria2 => {
            // hy2 inbound 不需要 server_name（sing-box 官方示例亦无此字段）；
            // 证书 CN 用 tag 本身，server_name 交由客户端从 URL 的 sni 决定（默认回落到 server）。
            let (crt, key) = ensure_self_signed_cert(&req.tag, &req.tag)?;
            Ok((
                json!({
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
                }),
                AddNodeMeta::Plain,
            ))
        }
        Protocol::Tuic => {
            let sni = req.server_name.clone().unwrap_or_else(|| "bing.com".into());
            let (crt, key) = ensure_self_signed_cert(&req.tag, &sni)?;
            Ok((
                json!({
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
                }),
                AddNodeMeta::Plain,
            ))
        }
        Protocol::Anytls => {
            let sni = req.server_name.clone().unwrap_or_else(|| "bing.com".into());
            let (crt, key) = ensure_self_signed_cert(&req.tag, &sni)?;
            Ok((
                json!({
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
                }),
                AddNodeMeta::Plain,
            ))
        }
        // 不给未知协议生成兜底 inbound：以前这里落成 direct，一个打错的协议名
        // 会静默生成一个开放的直连入站。宁可让 add 失败也不要生成非预期的监听。
        Protocol::Unknown => anyhow::bail!(
            "未知协议，无法生成 inbound；支持: vless-reality / vless-ws / vmess-ws / \
             trojan / shadowsocks / hysteria2 / tuic / anytls"
        ),
    }
}

/// 为 TLS 协议按需生成自签 cert/key 文件。使用 EC P-256（比 RSA 小很多，握手快）。
fn ensure_self_signed_cert(tag: &str, sni: &str) -> Result<(String, String)> {
    let base = Path::new(CERTS_DIR);
    std::fs::create_dir_all(base)
        .with_context(|| format!("创建证书目录 {} 失败", base.display()))?;
    let crt = base.join(format!("{}.crt", tag));
    let key = base.join(format!("{}.key", tag));
    if crt.exists() && key.exists() {
        return Ok((crt.display().to_string(), key.display().to_string()));
    }

    // 1. 生成 EC P-256 私钥
    let status = Command::new("openssl")
        .args([
            "ecparam",
            "-name",
            "prime256v1",
            "-genkey",
            "-noout",
            "-out",
        ])
        .arg(&key)
        .status()
        .with_context(|| "调用 openssl ecparam 失败（请确保已安装 openssl）")?;
    if !status.success() {
        anyhow::bail!("openssl 生成 EC 私钥失败 (tag={})", tag);
    }

    // 2. 用该私钥签一个 100 年有效的自签证书
    let status = Command::new("openssl")
        .args(["req", "-x509", "-new", "-key"])
        .arg(&key)
        .arg("-out")
        .arg(&crt)
        .args(["-days", "36500", "-nodes", "-subj"])
        .arg(format!("/CN={}", sni))
        .status()
        .with_context(|| "调用 openssl req 失败")?;
    if !status.success() {
        anyhow::bail!("openssl 生成自签证书失败 (tag={})", tag);
    }
    Ok((crt.display().to_string(), key.display().to_string()))
}

/// 调用 `sing-box generate reality-keypair`，返回 (private_key, public_key)。
/// 优先用调用方传入的 binary_path（来自 config.toml），再回落到常见安装位置。
fn generate_reality_keypair(binary_path: Option<&str>) -> Result<(String, String)> {
    let fallbacks = [
        "/etc/sing-box/bin/sing-box",
        "/usr/local/bin/sing-box",
        "/usr/bin/sing-box",
    ];
    let bin = binary_path
        .filter(|p| !p.trim().is_empty() && Path::new(p).exists())
        .map(str::to_string)
        .or_else(|| {
            fallbacks
                .iter()
                .find(|p| Path::new(p).exists())
                .map(|p| p.to_string())
        })
        .unwrap_or_else(|| "sing-box".to_string());
    let out = Command::new(&bin)
        .args(["generate", "reality-keypair"])
        .output()
        .with_context(|| format!("调用 {} generate reality-keypair 失败", bin))?;
    if !out.status.success() {
        anyhow::bail!(
            "sing-box generate reality-keypair 返回非零: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut priv_k = None;
    let mut pub_k = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(v) = line
            .strip_prefix("PrivateKey:")
            .or_else(|| line.strip_prefix("PrivateKey ="))
        {
            priv_k = Some(v.trim().to_string());
        } else if let Some(v) = line
            .strip_prefix("PublicKey:")
            .or_else(|| line.strip_prefix("PublicKey ="))
        {
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
    uuid::Uuid::new_v4()
        .simple()
        .to_string()
        .chars()
        .take(8)
        .collect()
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
    // listen 必须与 config.toml 的 grpc_addr 一致，否则 daemon 永远连不上统计接口。
    // 之前这里用 or_insert，管理员改了 grpc_addr 后 config.json 不会跟着走，
    // 只能靠 doctor 报不一致，没有自动修复路径。
    let previous = api.get("listen").and_then(Value::as_str).map(String::from);
    if previous.as_deref() != Some(grpc_addr) {
        if let Some(old) = previous {
            tracing::info!(
                old = %old,
                new = %grpc_addr,
                "v2ray_api.listen 与配置的 grpc_addr 不一致，已按 config.toml 更新"
            );
        }
        api.insert("listen".into(), Value::String(grpc_addr.to_string()));
    }
    let stats = api.entry("stats").or_insert_with(|| json!({}));
    let stats = ensure_object(stats);
    stats.insert("enabled".into(), Value::Bool(true));
    stats.insert(
        "users".into(),
        Value::Array(
            users
                .iter()
                .map(|u| Value::String(u.name.clone()))
                .collect(),
        ),
    );
}

/// 出站地址族策略。落到 sing-box 的 `route.default_domain_resolver.strategy`。
///
/// 注意用的不是 dial 字段 `domain_strategy`——那个 1.12.0 起弃用、1.14.0 已移除。
/// `default_domain_resolver` 是 1.12.0 引入的替代品，需要指向一个 DNS server tag。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutboundStrategy {
    /// 不写 `default_domain_resolver`，交给 sing-box 默认行为（系统解析器返回什么用什么）
    #[default]
    Auto,
    PreferIpv4,
    PreferIpv6,
    Ipv4Only,
    Ipv6Only,
}

impl OutboundStrategy {
    /// sing-box 侧的 strategy 取值；Auto 没有对应值。
    pub fn as_str(self) -> Option<&'static str> {
        match self {
            Self::Auto => None,
            Self::PreferIpv4 => Some("prefer_ipv4"),
            Self::PreferIpv6 => Some("prefer_ipv6"),
            Self::Ipv4Only => Some("ipv4_only"),
            Self::Ipv6Only => Some("ipv6_only"),
        }
    }

    /// CLI/TUI 用的短名。
    pub fn key(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::PreferIpv4 => "prefer4",
            Self::PreferIpv6 => "prefer6",
            Self::Ipv4Only => "v4only",
            Self::Ipv6Only => "v6only",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "自动 (跟随系统解析)",
            Self::PreferIpv4 => "优先 IPv4",
            Self::PreferIpv6 => "优先 IPv6",
            Self::Ipv4Only => "仅 IPv4",
            Self::Ipv6Only => "仅 IPv6",
        }
    }

    pub const ALL: [Self; 5] = [
        Self::Auto,
        Self::PreferIpv4,
        Self::PreferIpv6,
        Self::Ipv4Only,
        Self::Ipv6Only,
    ];

    /// TUI 里按一个键循环切换。
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    /// 同时接受短名和 sing-box 原值，容错几个常见写法。
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "auto" | "" | "default" => Some(Self::Auto),
            "prefer4" | "prefer_ipv4" | "ipv4" => Some(Self::PreferIpv4),
            "prefer6" | "prefer_ipv6" | "ipv6" => Some(Self::PreferIpv6),
            "v4only" | "ipv4_only" | "ipv4only" => Some(Self::Ipv4Only),
            "v6only" | "ipv6_only" | "ipv6only" => Some(Self::Ipv6Only),
            _ => None,
        }
    }
}

/// `default_domain_resolver` 指向的 DNS server tag；配置里没有 DNS server 时新建这个。
const LOCAL_DNS_TAG: &str = "local";

/// 读当前出站策略。`default_domain_resolver` 允许写成裸 tag 字符串，
/// 那种形式没带 strategy，等价于 Auto。
pub fn get_outbound_strategy(cfg: &Value) -> OutboundStrategy {
    cfg.get("route")
        .and_then(|r| r.get("default_domain_resolver"))
        .and_then(|d| d.get("strategy"))
        .and_then(Value::as_str)
        .and_then(OutboundStrategy::parse)
        .unwrap_or(OutboundStrategy::Auto)
}

/// 写出站策略。
///
/// - Auto：删掉 `default_domain_resolver`（`route` 变空也一并删，别留空壳）。
/// - 其余：确保有个可引用的 DNS server，再写 `{ server, strategy }`。
///
/// 已有的 `dns.servers` 一律不改写——里面可能是用户手配的上游，
/// 这里只借它的 tag 用。
pub fn set_outbound_strategy(cfg: &mut Value, strategy: OutboundStrategy) -> Result<()> {
    let Some(value) = strategy.as_str() else {
        let root = ensure_object(cfg);
        if let Some(Value::Object(route)) = root.get_mut("route") {
            route.remove("default_domain_resolver");
            if route.is_empty() {
                root.remove("route");
            }
        }
        return Ok(());
    };

    let server_tag = ensure_dns_server(cfg)?;
    let root = ensure_object(cfg);
    let route = root
        .entry("route")
        .or_insert_with(|| Value::Object(Map::new()));
    let route = route
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("route 字段不是对象"))?;
    route.insert(
        "default_domain_resolver".into(),
        json!({ "server": server_tag, "strategy": value }),
    );
    Ok(())
}

/// 返回一个可供 `default_domain_resolver.server` 引用的 DNS server tag。
/// 已有带 tag 的 server 就复用第一个；否则插入 `{ "type": "local", "tag": "local" }`
/// ——1.12.0 起的新式写法，旧的 `{ "address": ... }` 形式 1.14.0 已移除。
fn ensure_dns_server(cfg: &mut Value) -> Result<String> {
    let root = ensure_object(cfg);
    let dns = root
        .entry("dns")
        .or_insert_with(|| Value::Object(Map::new()));
    let dns = dns
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("dns 字段不是对象"))?;
    let servers = dns.entry("servers").or_insert_with(|| Value::Array(vec![]));
    let servers = servers
        .as_array_mut()
        .ok_or_else(|| anyhow::anyhow!("dns.servers 字段不是数组"))?;

    if let Some(tag) = servers
        .iter()
        .filter_map(|s| s.get("tag").and_then(Value::as_str))
        .find(|t| !t.trim().is_empty())
    {
        return Ok(tag.to_string());
    }
    servers.push(json!({ "type": "local", "tag": LOCAL_DNS_TAG }));
    Ok(LOCAL_DNS_TAG.to_string())
}

/// 删掉 1.13.0 已移除的老式特殊出站 `block` / `dns`。
///
/// 这两个 tag 在 1.11.0 弃用、1.13.0 移除，现在由路由规则动作
/// (`reject` / `hijack-dns`) 承担。留着的话新内核 `sing-box check` 直接报错，
/// 于是任何一次配置改写都会失败——所以顺手清掉。
/// 返回被删掉的 tag，供调用方提示。
pub fn strip_legacy_special_outbounds(cfg: &mut Value) -> Vec<String> {
    let mut removed = vec![];
    let Some(outbounds) = cfg.get_mut("outbounds").and_then(Value::as_array_mut) else {
        return removed;
    };
    outbounds.retain(|ob| {
        let ty = ob.get("type").and_then(Value::as_str).unwrap_or("");
        if ty == "block" || ty == "dns" {
            removed.push(
                ob.get("tag")
                    .and_then(Value::as_str)
                    .unwrap_or(ty)
                    .to_string(),
            );
            false
        } else {
            true
        }
    });
    removed
}

fn ensure_object(value: &mut Value) -> &mut Map<String, Value> {
    if !matches!(value, Value::Object(_)) {
        *value = Value::Object(Map::new());
    }
    match value {
        Value::Object(map) => map,
        _ => unreachable!("value 已被强制改写为 object"),
    }
}

#[cfg(test)]
mod tests {
    use super::{edit_node, sync_users, MetaOp};
    use crate::model::node::{EditNodeRequest, RelaySetting};
    use crate::model::user::User;
    use serde_json::json;

    fn sample_user(name: &str) -> User {
        User {
            name: name.into(),
            uuid: "de909d94-1d92-4a2f-9da8-c5b52a52282c".into(),
            password: "secret".into(),
            enabled: true,
            quota_gb: 0.0,
            used_up_bytes: 0,
            used_down_bytes: 0,
            last_live_up: 0,
            last_live_down: 0,
            reset_day: 0,
            last_reset_ym: String::new(),
            expire_at: String::new(),
            allow_all_nodes: true,
            created_at: "2026-01-01".into(),
            allowed_nodes: "[]".into(),
            sub_token: String::new(),
            traffic_multiplier: 1.0,
            tg_chat_id: 0,
            tg_bind_token: String::new(),
            tg_notify_quota_80: true,
            tg_notify_quota_90: true,
            tg_notify_quota_100: true,
            tg_schedule_enabled: true,
            tg_schedule_times: "[]".into(),
            tg_last_quota_level: 0,
            tg_last_schedule_dates: "{}".into(),
            auto_disabled: false,
        }
    }

    #[test]
    fn sync_users_recovers_non_object_root() {
        let mut cfg = json!(1);
        let users = vec![sample_user("alice")];

        let synced = sync_users(&mut cfg, &users, "127.0.0.1:18080");

        assert_eq!(synced, 0);
        assert_eq!(
            cfg["experimental"]["v2ray_api"]["listen"],
            "127.0.0.1:18080"
        );
        assert_eq!(cfg["experimental"]["v2ray_api"]["stats"]["enabled"], true);
        assert_eq!(
            cfg["experimental"]["v2ray_api"]["stats"]["users"][0],
            "alice"
        );
    }

    #[test]
    fn edit_node_does_not_inject_server_name_into_hy2() {
        let mut cfg = json!({
            "inbounds": [{
                "type": "hysteria2",
                "tag": "hy2",
                "listen": "::",
                "listen_port": 443,
                "users": [],
                "tls": {
                    "enabled": true,
                    "certificate_path": "/etc/sing-box/certs/hy2.crt",
                    "key_path": "/etc/sing-box/certs/hy2.key"
                }
            }]
        });

        let mut ops = Vec::new();
        edit_node(
            &mut cfg,
            &EditNodeRequest {
                tag: "hy2".into(),
                server_name: Some("www.apple.com".into()),
                ..Default::default()
            },
            &mut ops,
        )
        .unwrap();

        assert!(cfg["inbounds"][0]["tls"].get("server_name").is_none());
        assert!(ops.is_empty(), "无 port_reuse 改动时不该产生 MetaOp");
    }

    #[test]
    fn edit_node_port_reuse_pushes_meta_op_instead_of_writing_to_disk() {
        let mut cfg = json!({
            "inbounds": [{
                "type": "trojan",
                "tag": "trojan-1",
                "listen": "::",
                "listen_port": 443,
                "users": [],
                "tls": {
                    "enabled": true,
                    "server_name": "example.com",
                    "certificate_path": "/etc/sing-box/certs/trojan-1.crt",
                    "key_path": "/etc/sing-box/certs/trojan-1.key"
                }
            }]
        });
        let mut ops = Vec::new();
        edit_node(
            &mut cfg,
            &EditNodeRequest {
                tag: "trojan-1".into(),
                port_reuse: Some(true),
                ..Default::default()
            },
            &mut ops,
        )
        .unwrap();

        assert_eq!(cfg["inbounds"][0]["listen"], "127.0.0.1");
        assert_eq!(ops.len(), 1, "port_reuse 改动应入队一个 MetaOp");
        match &ops[0] {
            MetaOp::Set(tag, m) => {
                assert_eq!(tag, "trojan-1");
                assert!(m.port_reuse, "port_reuse=true 应被记录");
            }
            other => panic!("期望 MetaOp::Set，实际 {:?}", other),
        }
    }

    /// 中转是整组覆盖：设了就写进 meta，地址给空串就连端口一起清掉。
    #[test]
    fn edit_node_sets_and_clears_relay() {
        let mut cfg = json!({
            "inbounds": [{
                "type": "vless", "tag": "n1", "listen": "::", "listen_port": 4433, "users": []
            }]
        });

        let mut ops = Vec::new();
        edit_node(
            &mut cfg,
            &EditNodeRequest {
                tag: "n1".into(),
                relay: Some(RelaySetting {
                    host: "relay.example.com".into(),
                    port: Some(12345),
                }),
                ..Default::default()
            },
            &mut ops,
        )
        .unwrap();
        match &ops[0] {
            MetaOp::Set(tag, m) => {
                assert_eq!(tag, "n1");
                assert_eq!(m.relay_host.as_deref(), Some("relay.example.com"));
                assert_eq!(m.relay_port, Some(12345));
                assert_eq!(m.relay_label().as_deref(), Some("relay.example.com:12345"));
            }
            other => panic!("expected MetaOp::Set, got {:?}", other),
        }

        // 地址给空串 = 关闭中转，端口也要一起清，不能留半截状态
        let mut ops = Vec::new();
        edit_node(
            &mut cfg,
            &EditNodeRequest {
                tag: "n1".into(),
                relay: Some(RelaySetting::default()),
                ..Default::default()
            },
            &mut ops,
        )
        .unwrap();
        match &ops[0] {
            MetaOp::Set(_, m) => {
                assert!(m.relay_host.is_none());
                assert!(m.relay_port.is_none());
                assert!(m.relay_label().is_none());
            }
            other => panic!("expected MetaOp::Set, got {:?}", other),
        }
    }

    /// relay 为 None 表示"不改"，不应产生任何 meta 改动。
    #[test]
    fn edit_node_without_relay_field_leaves_meta_untouched() {
        let mut cfg = json!({
            "inbounds": [{
                "type": "vless", "tag": "n1", "listen": "::", "listen_port": 4433, "users": []
            }]
        });
        let mut ops = Vec::new();
        edit_node(
            &mut cfg,
            &EditNodeRequest {
                tag: "n1".into(),
                listen_port: Some(8443),
                ..Default::default()
            },
            &mut ops,
        )
        .unwrap();
        assert_eq!(cfg["inbounds"][0]["listen_port"], 8443);
        assert!(ops.is_empty(), "only changing the port must not touch meta");
    }

    #[test]
    fn sync_users_removes_stale_named_entries_but_keeps_default_user() {
        let mut cfg = json!({
            "inbounds": [{
                "type": "trojan",
                "tag": "trojan-1",
                "listen": "::",
                "listen_port": 443,
                "users": [
                    { "name": "alice", "password": "old-secret" },
                    { "name": "bob", "password": "stale-secret" },
                    { "name": "default", "password": "keep-me" }
                ],
                "tls": {
                    "enabled": true,
                    "server_name": "example.com",
                    "certificate_path": "/etc/sing-box/certs/trojan-1.crt",
                    "key_path": "/etc/sing-box/certs/trojan-1.key"
                }
            }]
        });
        let users = vec![sample_user("alice")];

        let synced = sync_users(&mut cfg, &users, "127.0.0.1:18080");
        let arr = cfg["inbounds"][0]["users"].as_array().unwrap();

        assert_eq!(synced, 1);
        assert_eq!(arr.len(), 2);
        assert!(arr.iter().any(|item| item["name"] == "alice"));
        assert!(arr.iter().any(|item| item["name"] == "default"));
        assert!(!arr.iter().any(|item| item["name"] == "bob"));
    }

    // ——— 出站地址族策略 ———

    use super::{
        get_outbound_strategy, set_outbound_strategy, strip_legacy_special_outbounds,
        OutboundStrategy,
    };

    /// 空配置上设策略要自己把 dns.servers 建起来，并且用 1.12.0 起的新式写法
    /// （`type`+`tag`），不能退回 1.14.0 已移除的 `address` 形式。
    #[test]
    fn set_strategy_creates_modern_local_dns_server() {
        let mut cfg = json!({ "inbounds": [], "outbounds": [] });
        set_outbound_strategy(&mut cfg, OutboundStrategy::Ipv4Only).unwrap();

        let servers = cfg["dns"]["servers"].as_array().unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0]["type"], "local");
        assert_eq!(servers[0]["tag"], "local");
        assert!(
            servers[0].get("address").is_none(),
            "不能用 1.14.0 已移除的 address 写法"
        );
        assert_eq!(
            cfg["route"]["default_domain_resolver"],
            json!({ "server": "local", "strategy": "ipv4_only" })
        );
    }

    /// 用的必须是 default_domain_resolver，而不是 1.14.0 已移除的 dial 字段
    /// domain_strategy——这是本功能的核心约束，专门钉一个测试。
    #[test]
    fn never_writes_the_removed_domain_strategy_field() {
        for s in OutboundStrategy::ALL {
            let mut cfg = json!({ "outbounds": [ { "type": "direct", "tag": "direct" } ] });
            set_outbound_strategy(&mut cfg, s).unwrap();
            let dumped = serde_json::to_string(&cfg).unwrap();
            assert!(
                !dumped.contains("domain_strategy"),
                "{:?} 写出了 domain_strategy: {}",
                s,
                dumped
            );
        }
    }

    /// 已有 DNS server 的配置只借 tag，不该被改写或追加。
    #[test]
    fn set_strategy_reuses_existing_dns_server_tag() {
        let mut cfg = json!({
            "dns": { "servers": [ { "type": "udp", "server": "1.1.1.1", "tag": "cloudflare" } ] },
            "outbounds": []
        });
        set_outbound_strategy(&mut cfg, OutboundStrategy::PreferIpv6).unwrap();

        let servers = cfg["dns"]["servers"].as_array().unwrap();
        assert_eq!(servers.len(), 1, "不该追加新的 DNS server");
        assert_eq!(servers[0]["server"], "1.1.1.1", "不该改写已有 server");
        assert_eq!(
            cfg["route"]["default_domain_resolver"]["server"],
            "cloudflare"
        );
        assert_eq!(
            cfg["route"]["default_domain_resolver"]["strategy"],
            "prefer_ipv6"
        );
    }

    /// 已有 server 但都没 tag 时无从引用，得自己补一个。
    #[test]
    fn set_strategy_appends_local_when_existing_servers_have_no_tag() {
        let mut cfg = json!({
            "dns": { "servers": [ { "type": "udp", "server": "1.1.1.1" } ] },
            "outbounds": []
        });
        set_outbound_strategy(&mut cfg, OutboundStrategy::Ipv6Only).unwrap();

        let servers = cfg["dns"]["servers"].as_array().unwrap();
        assert_eq!(servers.len(), 2);
        assert_eq!(cfg["route"]["default_domain_resolver"]["server"], "local");
    }

    #[test]
    fn strategy_round_trips_through_config() {
        for s in OutboundStrategy::ALL {
            let mut cfg = json!({ "outbounds": [] });
            set_outbound_strategy(&mut cfg, s).unwrap();
            assert_eq!(get_outbound_strategy(&cfg), s, "{:?} 没能读回来", s);
        }
    }

    /// 切回 Auto 要把字段删干净，不能留个空 route 壳子。
    #[test]
    fn auto_removes_the_resolver_and_empty_route() {
        let mut cfg = json!({ "outbounds": [] });
        set_outbound_strategy(&mut cfg, OutboundStrategy::Ipv4Only).unwrap();
        set_outbound_strategy(&mut cfg, OutboundStrategy::Auto).unwrap();

        assert_eq!(get_outbound_strategy(&cfg), OutboundStrategy::Auto);
        assert!(cfg.get("route").is_none(), "空 route 应该一起删掉");
    }

    /// route 里还有别的设置时只删 default_domain_resolver，别把 route 整段端走。
    #[test]
    fn auto_keeps_route_when_it_has_other_settings() {
        let mut cfg = json!({
            "route": { "final": "direct", "default_domain_resolver": { "server": "local", "strategy": "ipv4_only" } },
            "outbounds": []
        });
        set_outbound_strategy(&mut cfg, OutboundStrategy::Auto).unwrap();

        assert_eq!(cfg["route"]["final"], "direct");
        assert!(cfg["route"].get("default_domain_resolver").is_none());
    }

    /// default_domain_resolver 也允许写成裸 tag 字符串，那种形式没带 strategy。
    #[test]
    fn bare_tag_resolver_reads_as_auto() {
        let cfg = json!({ "route": { "default_domain_resolver": "local" } });
        assert_eq!(get_outbound_strategy(&cfg), OutboundStrategy::Auto);
    }

    #[test]
    fn parse_accepts_short_names_and_singbox_values() {
        assert_eq!(
            OutboundStrategy::parse("v4only"),
            Some(OutboundStrategy::Ipv4Only)
        );
        assert_eq!(
            OutboundStrategy::parse("IPv4_Only"),
            Some(OutboundStrategy::Ipv4Only)
        );
        assert_eq!(
            OutboundStrategy::parse("prefer-ipv6"),
            Some(OutboundStrategy::PreferIpv6)
        );
        assert_eq!(
            OutboundStrategy::parse("auto"),
            Some(OutboundStrategy::Auto)
        );
        assert_eq!(OutboundStrategy::parse("v5only"), None);
    }

    #[test]
    fn next_cycles_through_every_strategy_and_wraps() {
        let mut seen = vec![OutboundStrategy::Auto];
        let mut cur = OutboundStrategy::Auto;
        for _ in 0..OutboundStrategy::ALL.len() {
            cur = cur.next();
            if cur != OutboundStrategy::Auto {
                seen.push(cur);
            }
        }
        assert_eq!(cur, OutboundStrategy::Auto, "循环应该回到起点");
        assert_eq!(seen.len(), OutboundStrategy::ALL.len());
    }

    /// block / dns 特殊出站 1.13.0 已移除，留着会让 sing-box check 直接失败。
    #[test]
    fn strips_legacy_block_and_dns_outbounds() {
        let mut cfg = json!({
            "outbounds": [
                { "type": "direct", "tag": "direct" },
                { "type": "block", "tag": "block" },
                { "type": "dns", "tag": "dns-out" }
            ]
        });
        let removed = strip_legacy_special_outbounds(&mut cfg);

        assert_eq!(removed, vec!["block", "dns-out"]);
        let left = cfg["outbounds"].as_array().unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0]["tag"], "direct");
    }

    #[test]
    fn stripping_is_a_noop_on_a_clean_config() {
        let mut cfg = json!({ "outbounds": [ { "type": "direct", "tag": "direct" } ] });
        assert!(strip_legacy_special_outbounds(&mut cfg).is_empty());
        assert_eq!(cfg["outbounds"].as_array().unwrap().len(), 1);
    }
}
