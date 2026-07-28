use chrono::NaiveDate;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

pub const PROTOCOLS: [&str; 8] = [
    // 常用在前：vless（reality + ws） → hysteria2 → vmess-ws，其余按偏好
    "vless-reality",
    "vless-ws",
    "hysteria2",
    "vmess-ws",
    "trojan",
    "shadowsocks",
    "tuic",
    "anytls",
];

/// 中转两栏的标签。地址与端口分开填而不是写成 `ip:port`——
/// IPv6 字面量本身带冒号，混在一起没法无歧义地拆。
const RELAY_HOST_LABEL: &str = "中转地址 (例 1.2.3.4 / relay.com)";
const RELAY_PORT_LABEL: &str = "中转端口 (例 12345)";
/// 中转语义放这行，标签里塞不下
const RELAY_HINT: &str = "  中转：地址留空=不启用；只填地址则端口沿用节点端口；订阅按中转地址导出";

/// 节点表单里的逻辑字段，用来按协议动态组装 add/edit 表单。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NodeField {
    Tag,
    Protocol,
    Port,
    ServerName,
    Path,
    PortReuse,
    Ipv6,
    RelayHost,
    RelayPort,
}

/// 需要 TLS SNI 的协议（inbound tls.server_name 生效）。
/// 对照参考脚本 `20_protocol.sh`：只有 reality/trojan/tuic/anytls 真正用到 SNI；
/// hysteria2 / shadowsocks / *-ws 都不应该出现 server_name 字段。
pub fn protocol_uses_sni(p: &str) -> bool {
    matches!(p, "vless-reality" | "trojan" | "tuic" | "anytls")
}

/// 需要 WebSocket path 的协议。
pub fn protocol_uses_path(p: &str) -> bool {
    matches!(p, "vless-ws" | "vmess-ws")
}

/// 能通过 nginx stream SNI 分流做端口复用的协议（TCP + TLS-with-SNI）。
/// hy2/tuic 是 UDP QUIC，走不了 stream preread；ss / *-ws 没 TLS SNI 也不行。
pub fn protocol_supports_port_reuse(p: &str) -> bool {
    matches!(p, "vless-reality" | "trojan" | "anytls")
}

fn add_fields(protocol: &str) -> Vec<NodeField> {
    let mut v = vec![NodeField::Tag, NodeField::Protocol, NodeField::Port];
    if protocol_uses_sni(protocol) {
        v.push(NodeField::ServerName);
    }
    if protocol_uses_path(protocol) {
        v.push(NodeField::Path);
    }
    if protocol_supports_port_reuse(protocol) {
        v.push(NodeField::PortReuse);
    }
    v.push(NodeField::Ipv6);
    // 中转对所有协议都适用：只换订阅里导出的落点，不碰 inbound
    v.push(NodeField::RelayHost);
    v.push(NodeField::RelayPort);
    v
}

fn edit_fields(protocol: &str) -> Vec<NodeField> {
    let mut v = vec![NodeField::Port];
    if protocol_uses_sni(protocol) {
        v.push(NodeField::ServerName);
    }
    if protocol_uses_path(protocol) {
        v.push(NodeField::Path);
    }
    if protocol_supports_port_reuse(protocol) {
        v.push(NodeField::PortReuse);
    }
    v.push(NodeField::Ipv6);
    v.push(NodeField::RelayHost);
    v.push(NodeField::RelayPort);
    v
}

#[derive(Default)]
pub struct UserForm {
    pub name: String,
    pub quota: String,
    pub reset_day: String,
    pub expire: String,
    pub multiplier: String,
    pub focus: usize,
    pub error: Option<String>,
}

impl UserForm {
    pub fn new() -> Self {
        Self {
            multiplier: "2.0".into(),
            ..Default::default()
        }
    }
}

#[derive(Default)]
pub struct NodeForm {
    pub tag: String,
    pub protocol_idx: usize,
    pub port: String,
    pub server_name: String,
    pub path: String,
    pub port_reuse: bool,
    pub ipv6: bool,
    /// 中转机地址；空 = 不启用中转
    pub relay_host: String,
    /// 中转机端口；留空则沿用节点端口
    pub relay_port: String,
    pub focus: usize,
    pub error: Option<String>,
}

pub enum Modal {
    AddUser(UserForm),
    EditUser(UserEditForm),
    AddNode(NodeForm),
    EditNode(NodeEditForm),
    ConfirmDeleteUser(String),
    ConfirmDeleteNode(String),
    ConfirmResetUser(String),
    NodePicker(NodePicker),
    SubUrl {
        name: String,
        singbox: String,
        mihomo: String,
    },
    TokenManage {
        name: String,
        has_token: bool,
    },
    SelectRestore {
        files: Vec<String>,
        cursor: usize,
    },
}

#[derive(Default)]
pub struct UserEditForm {
    pub name: String, // 只读，用作定位
    pub quota: String,
    pub reset_day: String,
    pub expire: String,
    pub multiplier: String,
    pub focus: usize,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct NodeEditForm {
    pub tag: String,      // 只读，用作定位
    pub protocol: String, // 只读，用于渲染
    pub port: String,
    pub server_name: String,
    pub path: String,
    pub port_reuse: bool, // 端口复用：开启时订阅 URL 的端口固定 443
    pub ipv6: bool,
    pub relay_host: String,
    pub relay_port: String,
    pub focus: usize,
    pub error: Option<String>,
}

pub struct NodePicker {
    pub user: String,
    pub tags: Vec<String>,
    pub checked: Vec<bool>,
    pub cursor: usize,
    pub all: bool, // 对应 allow_all_nodes
}

pub enum ModalAction {
    None,
    Close,
    SubmitUser {
        name: String,
        quota: f64,
        reset_day: i64,
        expire: String,
        multiplier: f64,
    },
    SubmitUserEdit {
        name: String,
        quota: Option<f64>,
        reset_day: Option<i64>,
        expire: Option<String>,
        multiplier: Option<f64>,
    },
    SubmitNode {
        tag: String,
        protocol: String,
        port: u16,
        server_name: Option<String>,
        path: Option<String>,
        port_reuse: bool,
        ipv6: bool,
        relay: crate::model::node::RelaySetting,
    },
    SubmitNodeEdit {
        tag: String,
        port: Option<u16>,
        server_name: Option<String>,
        path: Option<String>,
        port_reuse: Option<bool>,
        ipv6: Option<bool>,
        relay: crate::model::node::RelaySetting,
    },
    DeleteUser(String),
    DeleteNode(String),
    ResetTraffic(String),
    SaveNodePicker {
        user: String,
        all: bool,
        tags: Vec<String>,
    },
    RegenToken(String),
    RevokeToken(String),
    RestoreBackup(String),
}

impl ModalAction {
    /// 是否是会改动数据（DB / config.json / 磁盘）的动作。
    /// 这类动作需要防重入，不能在上一次还没跑完时再提交一次。
    pub fn is_write(&self) -> bool {
        !matches!(self, ModalAction::None | ModalAction::Close)
    }
}

impl Modal {
    pub fn handle(&mut self, k: KeyEvent) -> ModalAction {
        if matches!(k.code, KeyCode::Esc) {
            return ModalAction::Close;
        }
        match self {
            Modal::AddUser(f) => handle_user(f, k),
            Modal::EditUser(f) => handle_user_edit(f, k),
            Modal::AddNode(f) => handle_node(f, k),
            Modal::EditNode(f) => handle_node_edit(f, k),
            Modal::ConfirmDeleteUser(name) => match k.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    ModalAction::DeleteUser(name.clone())
                }
                KeyCode::Char('n') | KeyCode::Char('N') => ModalAction::Close,
                _ => ModalAction::None,
            },
            Modal::ConfirmDeleteNode(tag) => match k.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    ModalAction::DeleteNode(tag.clone())
                }
                KeyCode::Char('n') | KeyCode::Char('N') => ModalAction::Close,
                _ => ModalAction::None,
            },
            Modal::ConfirmResetUser(name) => match k.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    ModalAction::ResetTraffic(name.clone())
                }
                KeyCode::Char('n') | KeyCode::Char('N') => ModalAction::Close,
                _ => ModalAction::None,
            },
            Modal::NodePicker(p) => handle_picker(p, k),
            Modal::SubUrl { .. } => match k.code {
                KeyCode::Enter | KeyCode::Char(' ') => ModalAction::Close,
                _ => ModalAction::None,
            },
            Modal::TokenManage { name, has_token } => match k.code {
                KeyCode::Char('g') | KeyCode::Char('G') => ModalAction::RegenToken(name.clone()),
                KeyCode::Char('v') | KeyCode::Char('V') if *has_token => {
                    ModalAction::RevokeToken(name.clone())
                }
                _ => ModalAction::None,
            },
            Modal::SelectRestore { files, cursor } => match k.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    *cursor = if *cursor == 0 {
                        files.len().saturating_sub(1)
                    } else {
                        *cursor - 1
                    };
                    ModalAction::None
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if !files.is_empty() {
                        *cursor = (*cursor + 1) % files.len();
                    }
                    ModalAction::None
                }
                KeyCode::Enter => {
                    if let Some(f) = files.get(*cursor) {
                        ModalAction::RestoreBackup(f.clone())
                    } else {
                        ModalAction::None
                    }
                }
                _ => ModalAction::None,
            },
        }
    }
}

fn handle_user_edit(f: &mut UserEditForm, k: KeyEvent) -> ModalAction {
    const FIELDS: usize = 4;
    f.error = None;
    match k.code {
        KeyCode::Tab | KeyCode::Down => {
            f.focus = (f.focus + 1) % FIELDS;
            ModalAction::None
        }
        KeyCode::BackTab | KeyCode::Up => {
            f.focus = if f.focus == 0 {
                FIELDS - 1
            } else {
                f.focus - 1
            };
            ModalAction::None
        }
        KeyCode::Enter => {
            let q = if f.quota.trim().is_empty() {
                None
            } else {
                match f.quota.trim().parse::<f64>() {
                    Ok(v) => Some(v),
                    Err(_) => {
                        f.error = Some("配额需为数字".into());
                        return ModalAction::None;
                    }
                }
            };
            let d = if f.reset_day.trim().is_empty() {
                None
            } else {
                match f.reset_day.trim().parse::<i64>() {
                    Ok(v) if v == 0 || v == 32 || (1..=31).contains(&v) => Some(v),
                    _ => {
                        f.error = Some("重置日需 0/1-31/32".into());
                        return ModalAction::None;
                    }
                }
            };
            let e = if f.expire.trim().is_empty() {
                None
            } else if f.expire.trim() == "-" {
                Some(String::new())
            }
            // 清为永久
            else if valid_date_text(f.expire.trim()) {
                Some(f.expire.trim().to_string())
            } else {
                f.error = Some("到期日格式需 YYYY-MM-DD".into());
                return ModalAction::None;
            };
            let m = if f.multiplier.trim().is_empty() {
                None
            } else {
                match f.multiplier.trim().parse::<f64>() {
                    Ok(v) if v >= 0.0 => Some(v),
                    _ => {
                        f.error = Some("倍率需为大于等于 0 的数字".into());
                        return ModalAction::None;
                    }
                }
            };
            ModalAction::SubmitUserEdit {
                name: f.name.clone(),
                quota: q,
                reset_day: d,
                expire: e,
                multiplier: m,
            }
        }
        KeyCode::Backspace => {
            user_edit_field(f).pop();
            ModalAction::None
        }
        KeyCode::Char(c) => {
            user_edit_field(f).push(c);
            ModalAction::None
        }
        _ => ModalAction::None,
    }
}

fn user_edit_field(f: &mut UserEditForm) -> &mut String {
    match f.focus {
        0 => &mut f.quota,
        1 => &mut f.reset_day,
        2 => &mut f.expire,
        _ => &mut f.multiplier,
    }
}

/// 解析表单里的中转设置。地址留空即关闭中转；端口留空表示沿用节点端口。
/// 返回 Err 时是给用户看的报错文案。
fn parse_relay(host_raw: &str, port_raw: &str) -> Result<crate::model::node::RelaySetting, String> {
    let host = host_raw.trim();
    let port_s = port_raw.trim();
    if host.is_empty() {
        // 只填端口没填地址属于填了一半，明确报错而不是静默丢弃
        if !port_s.is_empty() {
            return Err("填了中转端口就必须填中转 IP/域名".into());
        }
        return Ok(crate::model::node::RelaySetting::default());
    }
    if host.len() > 64 || host.chars().any(|c| c.is_whitespace()) {
        return Err("中转 IP/域名不合法（不能含空格，长度 ≤64）".into());
    }
    let port = if port_s.is_empty() {
        None
    } else {
        match port_s.parse::<u16>() {
            Ok(v) if v > 0 => Some(v),
            _ => return Err("中转端口需为 1-65535".into()),
        }
    };
    Ok(crate::model::node::RelaySetting {
        host: host.to_string(),
        port,
    })
}

fn handle_node_edit(f: &mut NodeEditForm, k: KeyEvent) -> ModalAction {
    let fields = edit_fields(&f.protocol);
    let n = fields.len().max(1);
    if f.focus >= n {
        f.focus = n - 1;
    }
    let focused = fields.get(f.focus).copied();
    f.error = None;
    match k.code {
        KeyCode::Tab | KeyCode::Down => {
            f.focus = (f.focus + 1) % n;
            ModalAction::None
        }
        KeyCode::BackTab | KeyCode::Up => {
            f.focus = if f.focus == 0 { n - 1 } else { f.focus - 1 };
            ModalAction::None
        }
        KeyCode::Left | KeyCode::Right | KeyCode::Char(' ')
            if focused == Some(NodeField::PortReuse) =>
        {
            f.port_reuse = !f.port_reuse;
            ModalAction::None
        }
        KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') if focused == Some(NodeField::Ipv6) => {
            f.ipv6 = !f.ipv6;
            ModalAction::None
        }
        KeyCode::Enter => {
            let port = if f.port.trim().is_empty() {
                None
            } else {
                match f.port.trim().parse::<u16>() {
                    Ok(v) if v > 0 => Some(v),
                    _ => {
                        f.error = Some("端口需为 1-65535".into());
                        return ModalAction::None;
                    }
                }
            };
            let sn = if protocol_uses_sni(&f.protocol) && !f.server_name.trim().is_empty() {
                Some(f.server_name.trim().to_string())
            } else {
                None
            };
            let pa = if protocol_uses_path(&f.protocol) && !f.path.trim().is_empty() {
                Some(f.path.trim().to_string())
            } else {
                None
            };
            // 只有可复用的协议才回传 port_reuse 开关
            let pr = if protocol_supports_port_reuse(&f.protocol) {
                Some(f.port_reuse)
            } else {
                None
            };
            let relay = match parse_relay(&f.relay_host, &f.relay_port) {
                Ok(r) => r,
                Err(msg) => {
                    f.error = Some(msg);
                    return ModalAction::None;
                }
            };
            ModalAction::SubmitNodeEdit {
                tag: f.tag.clone(),
                port,
                server_name: sn,
                path: pa,
                port_reuse: pr,
                ipv6: Some(f.ipv6),
                relay,
            }
        }
        KeyCode::Backspace
            if focused != Some(NodeField::PortReuse) && focused != Some(NodeField::Ipv6) =>
        {
            if let Some(s) = node_edit_field_mut(f, focused) {
                s.pop();
            }
            ModalAction::None
        }
        KeyCode::Char(c)
            if focused != Some(NodeField::PortReuse) && focused != Some(NodeField::Ipv6) =>
        {
            if let Some(s) = node_edit_field_mut(f, focused) {
                s.push(c);
            }
            ModalAction::None
        }
        _ => ModalAction::None,
    }
}

fn node_edit_field_mut(f: &mut NodeEditForm, which: Option<NodeField>) -> Option<&mut String> {
    match which? {
        NodeField::Port => Some(&mut f.port),
        NodeField::ServerName => Some(&mut f.server_name),
        NodeField::Path => Some(&mut f.path),
        NodeField::RelayHost => Some(&mut f.relay_host),
        NodeField::RelayPort => Some(&mut f.relay_port),
        _ => None,
    }
}

fn handle_picker(p: &mut NodePicker, k: KeyEvent) -> ModalAction {
    let len = p.tags.len();
    match k.code {
        KeyCode::Up | KeyCode::Char('k') => {
            if len > 0 {
                p.cursor = if p.cursor == 0 { len - 1 } else { p.cursor - 1 };
            }
            ModalAction::None
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if len > 0 {
                p.cursor = (p.cursor + 1) % len;
            }
            ModalAction::None
        }
        KeyCode::Char(' ') => {
            if let Some(v) = p.checked.get_mut(p.cursor) {
                *v = !*v;
            }
            p.all = false;
            ModalAction::None
        }
        KeyCode::Char('a') => {
            // 切换 all
            p.all = !p.all;
            if p.all {
                for v in p.checked.iter_mut() {
                    *v = false;
                }
            }
            ModalAction::None
        }
        KeyCode::Enter => {
            let tags: Vec<String> = if p.all {
                vec![]
            } else {
                p.tags
                    .iter()
                    .zip(p.checked.iter())
                    .filter_map(|(t, c)| if *c { Some(t.clone()) } else { None })
                    .collect()
            };
            ModalAction::SaveNodePicker {
                user: p.user.clone(),
                all: p.all,
                tags,
            }
        }
        _ => ModalAction::None,
    }
}

fn handle_user(f: &mut UserForm, k: KeyEvent) -> ModalAction {
    const FIELDS: usize = 5;
    f.error = None;
    match k.code {
        KeyCode::Tab | KeyCode::Down => {
            f.focus = (f.focus + 1) % FIELDS;
            ModalAction::None
        }
        KeyCode::BackTab | KeyCode::Up => {
            f.focus = if f.focus == 0 {
                FIELDS - 1
            } else {
                f.focus - 1
            };
            ModalAction::None
        }
        KeyCode::Enter => {
            let name = f.name.trim().to_string();
            if name.is_empty() {
                f.error = Some("用户名必填".into());
                return ModalAction::None;
            }
            let quota: f64 = if f.quota.trim().is_empty() {
                0.0
            } else {
                match f.quota.trim().parse() {
                    Ok(v) => v,
                    Err(_) => {
                        f.error = Some("配额需为数字(GB)，0=不限".into());
                        return ModalAction::None;
                    }
                }
            };
            let reset_day: i64 = if f.reset_day.trim().is_empty() {
                0
            } else {
                match f.reset_day.trim().parse::<i64>() {
                    Ok(v) if v == 0 || v == 32 || (1..=31).contains(&v) => v,
                    _ => {
                        f.error = Some("重置日需 0/1-31/32".into());
                        return ModalAction::None;
                    }
                }
            };
            let expire = f.expire.trim().to_string();
            if !expire.is_empty() && !valid_date_text(&expire) {
                f.error = Some("到期日格式需 YYYY-MM-DD".into());
                return ModalAction::None;
            }
            let multiplier: f64 = if f.multiplier.trim().is_empty() {
                2.0
            } else {
                match f.multiplier.trim().parse() {
                    Ok(v) if v >= 0.0 => v,
                    _ => {
                        f.error = Some("倍率需为大于等于 0 的数字".into());
                        return ModalAction::None;
                    }
                }
            };
            ModalAction::SubmitUser {
                name,
                quota,
                reset_day,
                expire,
                multiplier,
            }
        }
        KeyCode::Backspace => {
            user_field(f).pop();
            ModalAction::None
        }
        KeyCode::Char(c) => {
            user_field(f).push(c);
            ModalAction::None
        }
        _ => ModalAction::None,
    }
}

fn valid_date_text(text: &str) -> bool {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok()
}

fn user_field(f: &mut UserForm) -> &mut String {
    match f.focus {
        0 => &mut f.name,
        1 => &mut f.quota,
        2 => &mut f.reset_day,
        3 => &mut f.expire,
        _ => &mut f.multiplier,
    }
}

fn handle_node(f: &mut NodeForm, k: KeyEvent) -> ModalAction {
    let fields = add_fields(PROTOCOLS[f.protocol_idx]);
    let n = fields.len();
    if f.focus >= n {
        f.focus = n - 1;
    }
    let focused = fields[f.focus];
    f.error = None;
    match k.code {
        KeyCode::Tab | KeyCode::Down => {
            f.focus = (f.focus + 1) % n;
            ModalAction::None
        }
        KeyCode::BackTab | KeyCode::Up => {
            f.focus = if f.focus == 0 { n - 1 } else { f.focus - 1 };
            ModalAction::None
        }
        KeyCode::Left if focused == NodeField::Protocol => {
            f.protocol_idx = if f.protocol_idx == 0 {
                PROTOCOLS.len() - 1
            } else {
                f.protocol_idx - 1
            };
            ModalAction::None
        }
        KeyCode::Right if focused == NodeField::Protocol => {
            f.protocol_idx = (f.protocol_idx + 1) % PROTOCOLS.len();
            ModalAction::None
        }
        KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') if focused == NodeField::PortReuse => {
            f.port_reuse = !f.port_reuse;
            ModalAction::None
        }
        KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') if focused == NodeField::Ipv6 => {
            f.ipv6 = !f.ipv6;
            ModalAction::None
        }
        KeyCode::Enter => {
            let tag = f.tag.trim().to_string();
            if tag.is_empty() {
                f.error = Some("tag 必填".into());
                return ModalAction::None;
            }
            let port: u16 = match f.port.trim().parse() {
                Ok(v) if v > 0 => v,
                _ => {
                    f.error = Some("端口需为 1-65535".into());
                    return ModalAction::None;
                }
            };
            let protocol = PROTOCOLS[f.protocol_idx].to_string();
            // 只在协议实际需要时回传对应字段，避免把 server_name / path 塞进不该有的协议。
            let sn = if protocol_uses_sni(&protocol) && !f.server_name.trim().is_empty() {
                Some(f.server_name.trim().to_string())
            } else {
                None
            };
            let path = if protocol_uses_path(&protocol) && !f.path.trim().is_empty() {
                Some(f.path.trim().to_string())
            } else {
                None
            };
            let reuse = protocol_supports_port_reuse(&protocol) && f.port_reuse;
            let relay = match parse_relay(&f.relay_host, &f.relay_port) {
                Ok(r) => r,
                Err(msg) => {
                    f.error = Some(msg);
                    return ModalAction::None;
                }
            };
            ModalAction::SubmitNode {
                tag,
                protocol,
                port,
                server_name: sn,
                path,
                port_reuse: reuse,
                ipv6: f.ipv6,
                relay,
            }
        }
        KeyCode::Backspace
            if !matches!(
                focused,
                NodeField::Protocol | NodeField::PortReuse | NodeField::Ipv6
            ) =>
        {
            if let Some(s) = node_field_mut(f, focused) {
                s.pop();
            }
            ModalAction::None
        }
        KeyCode::Char(c)
            if !matches!(
                focused,
                NodeField::Protocol | NodeField::PortReuse | NodeField::Ipv6
            ) =>
        {
            if let Some(s) = node_field_mut(f, focused) {
                s.push(c);
            }
            ModalAction::None
        }
        _ => ModalAction::None,
    }
}

fn node_field_mut(f: &mut NodeForm, which: NodeField) -> Option<&mut String> {
    match which {
        NodeField::Tag => Some(&mut f.tag),
        NodeField::Port => Some(&mut f.port),
        NodeField::ServerName => Some(&mut f.server_name),
        NodeField::Path => Some(&mut f.path),
        NodeField::RelayHost => Some(&mut f.relay_host),
        NodeField::RelayPort => Some(&mut f.relay_port),
        NodeField::Protocol | NodeField::PortReuse | NodeField::Ipv6 => None,
    }
}

pub fn render(f: &mut Frame, area: Rect, modal: &Modal) {
    // 节点表单的行数随协议和字段数变化，高度必须按实际内容算。
    // 以前这里写死 16 行，加字段就会被无声裁掉——中转两栏正是这么丢的。
    let node_panel = match modal {
        Modal::AddNode(form) => Some((node_lines(form), " 添加节点 ")),
        Modal::EditNode(form) => Some((node_edit_lines(form), " 编辑节点 ")),
        _ => None,
    };
    if let Some((lines, title)) = node_panel {
        let pop = centered(area, 76, lines.len() as u16 + 2);
        f.render_widget(Clear, pop);
        f.render_widget(
            Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(title)),
            pop,
        );
        return;
    }

    let pop = centered(area, 62, 16);
    f.render_widget(Clear, pop);
    match modal {
        Modal::AddUser(form) => render_user(f, pop, form, "添加用户"),
        Modal::EditUser(form) => render_user_edit(f, pop, form),
        // 上面已提前返回
        Modal::AddNode(_) | Modal::EditNode(_) => {}
        Modal::ConfirmDeleteUser(name) => render_confirm(f, pop, " 确认删除用户 ", name),
        Modal::ConfirmDeleteNode(tag) => render_confirm(f, pop, " 确认删除节点 ", tag),
        Modal::ConfirmResetUser(name) => render_reset_confirm(f, pop, name),
        Modal::NodePicker(p) => {
            render_picker(f, centered(area, 62, (p.tags.len() as u16 + 8).min(20)), p)
        }
        Modal::SubUrl {
            name,
            singbox,
            mihomo,
        } => {
            // URL 可能很长，modal 宽度用 min(屏宽-4, max(url长度+8, 62))
            let max_len = singbox.len().max(mihomo.len()) as u16 + 8;
            let w = max_len.max(62).min(area.width.saturating_sub(4));
            render_sub_url(f, centered(area, w, 12), name, singbox, mihomo);
        }
        Modal::TokenManage { name, has_token } => {
            render_token_manage(f, centered(area, 62, 10), name, *has_token);
        }
        Modal::SelectRestore { files, cursor } => {
            render_select_restore(f, centered(area, 62, 16), files, *cursor);
        }
    }
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(area.height.saturating_sub(h) / 2),
            Constraint::Length(h),
            Constraint::Min(0),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(area.width.saturating_sub(w) / 2),
            Constraint::Length(w),
            Constraint::Min(0),
        ])
        .split(v[1])[1]
}

fn render_user(f: &mut Frame, area: Rect, form: &UserForm, title: &str) {
    let labels = [
        "用户名",
        "配额 GB (0=不限)",
        "重置日 (1-31/32/0)",
        "到期 (YYYY-MM-DD, 例: 2026-12-31)",
        "流量倍率 (双倍=2.0, 单倍=1.0)",
    ];
    let vals = [
        &form.name,
        &form.quota,
        &form.reset_day,
        &form.expire,
        &form.multiplier,
    ];
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));
    for (i, (label, val)) in labels.iter().zip(vals).enumerate() {
        let style = if i == form.focus {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            Style::default().fg(Color::White)
        };
        let cursor = if i == form.focus { "_" } else { "" };
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {:<22}", label),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(format!(" {}{}  ", val, cursor), style),
        ]));
        lines.push(Line::from(""));
    }
    if let Some(e) = &form.error {
        lines.push(Line::from(Span::styled(
            format!("  ! {}", e),
            Style::default().fg(Color::Red),
        )));
    }
    lines.push(Line::from(Span::styled(
        "  Tab/↑↓ 切换   Enter 提交   Esc 取消   (留空使用默认值)",
        Style::default().fg(Color::DarkGray),
    )));
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {} ", title)),
        ),
        area,
    );
}

fn render_user_edit(f: &mut Frame, area: Rect, form: &UserEditForm) {
    let labels = [
        "配额 GB (留空不改)",
        "重置日 (留空不改)",
        "到期 (留空不改, - 清为永久, 例: 2026-12-31)",
        "流量倍率 (留空不改, 双倍=2.0)",
    ];
    let vals = [&form.quota, &form.reset_day, &form.expire, &form.multiplier];
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!("  用户: {}  （name 不可改，删掉重建）", form.name),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
    for (i, (label, val)) in labels.iter().zip(vals).enumerate() {
        let style = if i == form.focus {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            Style::default().fg(Color::White)
        };
        let cursor = if i == form.focus { "_" } else { "" };
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {:<22}", label),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(format!(" {}{}  ", val, cursor), style),
        ]));
        lines.push(Line::from(""));
    }
    if let Some(e) = &form.error {
        lines.push(Line::from(Span::styled(
            format!("  ! {}", e),
            Style::default().fg(Color::Red),
        )));
    }
    lines.push(Line::from(Span::styled(
        "  Tab/↑↓ 切换   Enter 保存   Esc 取消",
        Style::default().fg(Color::DarkGray),
    )));
    f.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" 编辑用户 ")),
        area,
    );
}

fn node_lines(form: &NodeForm) -> Vec<Line<'static>> {
    let protocol = PROTOCOLS[form.protocol_idx];
    let fields = add_fields(protocol);
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));
    for (i, field) in fields.iter().enumerate() {
        let (label, val): (&str, String) = match field {
            NodeField::Tag => ("Tag *必填", form.tag.clone()),
            NodeField::Protocol => ("协议 (←/→ 切换)", format!("◀ {} ▶", protocol)),
            NodeField::Port => ("端口 *必填 (默认 443)", form.port.clone()),
            NodeField::ServerName => ("server_name (SNI)", form.server_name.clone()),
            NodeField::Path => ("path (留空=默认)", form.path.clone()),
            NodeField::PortReuse => (
                "端口复用 (Space/←→ 切换)",
                format!("◀ {} ▶", if form.port_reuse { "开" } else { "关" }),
            ),
            NodeField::Ipv6 => (
                "订阅优先 IPv6 (Space/←→ 切换)",
                format!("◀ {} ▶", if form.ipv6 { "开" } else { "关" }),
            ),
            NodeField::RelayHost => (RELAY_HOST_LABEL, form.relay_host.clone()),
            NodeField::RelayPort => (RELAY_PORT_LABEL, form.relay_port.clone()),
        };
        let style = if i == form.focus {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            Style::default().fg(Color::White)
        };
        let cursor = if i == form.focus
            && !matches!(
                *field,
                NodeField::Protocol | NodeField::PortReuse | NodeField::Ipv6
            ) {
            "_"
        } else {
            ""
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {:<24}", label),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(format!(" {}{}  ", val, cursor), style),
        ]));
        lines.push(Line::from(""));
    }
    if let Some(e) = &form.error {
        lines.push(Line::from(Span::styled(
            format!("  ! {}", e),
            Style::default().fg(Color::Red),
        )));
    }
    let hint = match protocol {
        "vless-reality" => {
            "  reality: private_key/short_id 自动生成；server_name 同时作为 handshake 目标"
        }
        "vless-ws" | "vmess-ws" => "  ws: 后端不启 TLS，建议前挂 nginx/caddy 终结 TLS",
        "shadowsocks" => "  shadowsocks-2022：密钥自动生成，无 SNI / path 字段",
        "hysteria2" => "  hysteria2: 无 server_name / path；证书 CN=tag，客户端订阅走 insecure=1",
        "trojan" | "tuic" | "anytls" => {
            "  自签证书 CN=server_name；客户端订阅自动带 allowInsecure/insecure"
        }
        _ => "",
    };
    if !hint.is_empty() {
        lines.push(Line::from(Span::styled(
            hint,
            Style::default().fg(Color::DarkGray),
        )));
    }
    if protocol_supports_port_reuse(protocol) {
        lines.push(Line::from(Span::styled(
            "  端口复用开启：listen→127.0.0.1，订阅端口写 443；需手动配 nginx stream SNI 分流",
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines.push(Line::from(Span::styled(
        RELAY_HINT,
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(Span::styled(
        "  Tab/↑↓ 切换   ←/→ 选协议   Enter 提交   Esc 取消",
        Style::default().fg(Color::DarkGray),
    )));
    lines
}

fn node_edit_lines(form: &NodeEditForm) -> Vec<Line<'static>> {
    let fields = edit_fields(&form.protocol);
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!(
            "  Tag: {}   协议: {}   （tag/协议不可改，删掉重建）",
            form.tag, form.protocol
        ),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
    if fields.is_empty() {
        lines.push(Line::from(Span::styled(
            "  （该协议没有可编辑字段，删掉重建）",
            Style::default().fg(Color::DarkGray),
        )));
    }
    for (i, field) in fields.iter().enumerate() {
        let (label, val): (&str, String) = match field {
            NodeField::Port => ("端口 (留空不改)", form.port.clone()),
            NodeField::ServerName => ("server_name (留空不改)", form.server_name.clone()),
            NodeField::Path => ("path (留空不改)", form.path.clone()),
            NodeField::PortReuse => (
                "端口复用 (Space/←→ 切换)",
                format!("◀ {} ▶", if form.port_reuse { "开" } else { "关" }),
            ),
            NodeField::Ipv6 => (
                "订阅优先 IPv6 (Space/←→ 切换)",
                format!("◀ {} ▶", if form.ipv6 { "开" } else { "关" }),
            ),
            NodeField::RelayHost => (RELAY_HOST_LABEL, form.relay_host.clone()),
            NodeField::RelayPort => (RELAY_PORT_LABEL, form.relay_port.clone()),
            // 不写 `_ => continue`：漏掉新字段时要编译报错，而不是让它在界面上
            // 静默消失（中转两栏第一次加进来时就是这么丢的）。
            NodeField::Tag | NodeField::Protocol => continue,
        };
        let style = if i == form.focus {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            Style::default().fg(Color::White)
        };
        let cursor =
            if i == form.focus && *field != NodeField::PortReuse && *field != NodeField::Ipv6 {
                "_"
            } else {
                ""
            };
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {:<28}", label),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(format!(" {}{}  ", val, cursor), style),
        ]));
        lines.push(Line::from(""));
    }
    if protocol_supports_port_reuse(&form.protocol) {
        lines.push(Line::from(Span::styled(
            "  端口复用开启后：listen 改为 127.0.0.1，订阅端口写 443；",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(Span::styled(
            "  你需要手动在 nginx stream 里用 ssl_preread 做 SNI 分流（详见 README）。",
            Style::default().fg(Color::DarkGray),
        )));
    }
    if let Some(e) = &form.error {
        lines.push(Line::from(Span::styled(
            format!("  ! {}", e),
            Style::default().fg(Color::Red),
        )));
    }
    lines.push(Line::from(Span::styled(
        RELAY_HINT,
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(Span::styled(
        "  Tab/↑↓ 切换   Enter 保存   Esc 取消",
        Style::default().fg(Color::DarkGray),
    )));
    lines
}

fn render_confirm(f: &mut Frame, area: Rect, title: &str, target: &str) {
    let text = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  删除 '{}'？此操作不可撤销", target),
            Style::default().fg(Color::Red),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  [Y/Enter] 确认    [N/Esc] 取消",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    f.render_widget(
        Paragraph::new(text)
            .alignment(Alignment::Left)
            .block(Block::default().borders(Borders::ALL).title(title)),
        area,
    );
}

fn render_reset_confirm(f: &mut Frame, area: Rect, name: &str) {
    let text = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  重置 '{}' 的流量？", name),
            Style::default().fg(Color::Yellow),
        )),
        Line::from(Span::styled(
            "  只清零已用流量，不会改动月重置日期",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  [Y/Enter] 确认    [N/Esc] 取消",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    f.render_widget(
        Paragraph::new(text).alignment(Alignment::Left).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" 确认重置流量 "),
        ),
        area,
    );
}

fn render_token_manage(f: &mut Frame, area: Rect, name: &str, has_token: bool) {
    f.render_widget(Clear, area);
    let mut lines: Vec<Line> = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!(
                "  用户: {}   当前: {}",
                name,
                if has_token {
                    "● 订阅已开启"
                } else {
                    "○ 订阅已关闭"
                }
            ),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  [g]  重新生成 token（老 URL 立即失效）",
            Style::default().fg(Color::White),
        )),
    ];
    if has_token {
        lines.push(Line::from(Span::styled(
            "  [v]  撤销 token（关闭订阅，/sub/ 返回 404）",
            Style::default().fg(Color::White),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "  (已关闭状态，[g] 重新生成即可恢复)",
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  [Esc] 取消",
        Style::default().fg(Color::DarkGray),
    )));
    f.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" Token 管理 ")),
        area,
    );
}

fn render_sub_url(f: &mut Frame, area: Rect, name: &str, singbox: &str, mihomo: &str) {
    f.render_widget(Clear, area);
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  用户: {}", name),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  sing-box / v2rayN:",
            Style::default().fg(Color::Cyan),
        )),
        Line::from(Span::styled(
            format!("    {}", singbox),
            Style::default().fg(Color::White),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  mihomo / Clash-meta:",
            Style::default().fg(Color::Cyan),
        )),
        Line::from(Span::styled(
            format!("    {}", mihomo),
            Style::default().fg(Color::White),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  终端里鼠标选中即可复制；按 Esc/Enter 关闭",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    f.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" 订阅 URL ")),
        area,
    );
}

fn render_picker(f: &mut Frame, area: Rect, p: &NodePicker) {
    f.render_widget(Clear, area);
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!(
            "  用户: {}   当前: {}",
            p.user,
            if p.all { "全部节点" } else { "按列表" }
        ),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
    if p.tags.is_empty() {
        lines.push(Line::from(Span::styled(
            "  （没有节点，先去节点页按 [a] 添加）",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for (i, t) in p.tags.iter().enumerate() {
            let mark = if p.all {
                "[*]"
            } else if p.checked.get(i).copied().unwrap_or(false) {
                "[x]"
            } else {
                "[ ]"
            };
            let style = if i == p.cursor {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default().fg(Color::White)
            };
            lines.push(Line::from(Span::styled(format!("  {} {}", mark, t), style)));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  ↑↓/jk 选择   Space 勾选   a 切换全部   Enter 保存   Esc 取消",
        Style::default().fg(Color::DarkGray),
    )));
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" 分配可用节点 "),
        ),
        area,
    );
}

fn render_select_restore(f: &mut Frame, area: Rect, files: &[String], cursor: usize) {
    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        "  选择要恢复的备份",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    if files.is_empty() {
        lines.push(Line::from(Span::styled(
            "  没有找到任何备份",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for (i, file) in files.iter().enumerate() {
            let style = if i == cursor {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default().fg(Color::White)
            };
            lines.push(Line::from(vec![
                Span::raw(if i == cursor { " > " } else { "   " }),
                Span::styled(format!("{:<40}", file), style),
            ]));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  ↑↓ 选择   Enter 确认恢复   Esc 取消",
        Style::default().fg(Color::DarkGray),
    )));

    f.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" 恢复备份 ")),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 表单里出现的每个字段都必须真的画出来。
    ///
    /// 中转两栏第一次加进来时，`add_fields`/`edit_fields` 里有、渲染的 match
    /// 却因为末尾的 `_ => continue` 把它们跳过了：字段能聚焦、能提交，就是看不见。
    /// 这里对每个协议逐字段核对渲染结果，防止再犯。
    #[test]
    fn every_field_in_the_form_is_rendered() {
        for (idx, protocol) in PROTOCOLS.iter().enumerate() {
            let add = NodeForm {
                protocol_idx: idx,
                ..Default::default()
            };
            let rendered = lines_to_text(&node_lines(&add));
            for field in add_fields(protocol) {
                let label = field_label(field);
                assert!(
                    rendered.contains(label),
                    "添加表单({protocol}) 少画了字段: {label}"
                );
            }

            let edit = NodeEditForm {
                protocol: protocol.to_string(),
                ..Default::default()
            };
            let rendered = lines_to_text(&node_edit_lines(&edit));
            for field in edit_fields(protocol) {
                let label = field_label(field);
                assert!(
                    rendered.contains(label),
                    "编辑表单({protocol}) 少画了字段: {label}"
                );
            }
        }
    }

    /// 中转对所有协议都该出现，且添加/编辑两处都有。
    #[test]
    fn relay_fields_present_for_every_protocol() {
        for (idx, protocol) in PROTOCOLS.iter().enumerate() {
            assert!(add_fields(protocol).contains(&NodeField::RelayHost));
            assert!(add_fields(protocol).contains(&NodeField::RelayPort));
            assert!(edit_fields(protocol).contains(&NodeField::RelayHost));
            assert!(edit_fields(protocol).contains(&NodeField::RelayPort));

            let add = NodeForm {
                protocol_idx: idx,
                ..Default::default()
            };
            assert!(lines_to_text(&node_lines(&add)).contains(RELAY_HOST_LABEL));
            let edit = NodeEditForm {
                protocol: protocol.to_string(),
                ..Default::default()
            };
            assert!(lines_to_text(&node_edit_lines(&edit)).contains(RELAY_PORT_LABEL));
        }
    }

    fn lines_to_text(lines: &[Line<'static>]) -> String {
        lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|sp| sp.content.to_string()))
            .collect::<Vec<_>>()
            .join("")
    }

    fn field_label(field: NodeField) -> &'static str {
        match field {
            NodeField::Tag => "Tag",
            NodeField::Protocol => "协议",
            NodeField::Port => "端口",
            NodeField::ServerName => "server_name",
            NodeField::Path => "path",
            NodeField::PortReuse => "端口复用",
            NodeField::Ipv6 => "订阅优先 IPv6",
            NodeField::RelayHost => RELAY_HOST_LABEL,
            NodeField::RelayPort => RELAY_PORT_LABEL,
        }
    }

    #[test]
    fn parse_relay_accepts_host_only_and_rejects_port_only() {
        assert_eq!(
            parse_relay("", "").unwrap(),
            crate::model::node::RelaySetting::default()
        );
        let r = parse_relay(" relay.com ", "").unwrap();
        assert_eq!(r.host, "relay.com");
        assert_eq!(r.port, None);
        let r = parse_relay("1.2.3.4", "12345").unwrap();
        assert_eq!(r.port, Some(12345));
        // 只填端口不填地址是填了一半，应报错而不是静默丢弃
        assert!(parse_relay("", "12345").is_err());
        assert!(parse_relay("1.2.3.4", "0").is_err());
        assert!(parse_relay("1.2.3.4", "abc").is_err());
        assert!(parse_relay("a b", "").is_err());
    }
}
