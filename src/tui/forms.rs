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

/// 节点表单里的逻辑字段，用来按协议动态组装 add/edit 表单。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NodeField { Tag, Protocol, Port, ServerName, Path, PortReuse }

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
    if protocol_uses_sni(protocol)  { v.push(NodeField::ServerName); }
    if protocol_uses_path(protocol) { v.push(NodeField::Path); }
    if protocol_supports_port_reuse(protocol) { v.push(NodeField::PortReuse); }
    v
}

fn edit_fields(protocol: &str) -> Vec<NodeField> {
    let mut v = vec![NodeField::Port];
    if protocol_uses_sni(protocol)  { v.push(NodeField::ServerName); }
    if protocol_uses_path(protocol) { v.push(NodeField::Path); }
    if protocol_supports_port_reuse(protocol) { v.push(NodeField::PortReuse); }
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
    NodePicker(NodePicker),
    SubUrl { name: String, singbox: String, mihomo: String },
    TokenManage { name: String, has_token: bool },
}

#[derive(Default)]
pub struct UserEditForm {
    pub name: String,          // 只读，用作定位
    pub quota: String,
    pub reset_day: String,
    pub expire: String,
    pub multiplier: String,
    pub focus: usize,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct NodeEditForm {
    pub tag: String,           // 只读，用作定位
    pub protocol: String,      // 只读，用于渲染
    pub port: String,
    pub server_name: String,
    pub path: String,
    pub port_reuse: bool,      // 端口复用：开启时订阅 URL 的端口固定 443
    pub focus: usize,
    pub error: Option<String>,
}

pub struct NodePicker {
    pub user: String,
    pub tags: Vec<String>,
    pub checked: Vec<bool>,
    pub cursor: usize,
    pub all: bool,      // 对应 allow_all_nodes
}

pub enum ModalAction {
    None,
    Close,
    SubmitUser { name: String, quota: f64, reset_day: i64, expire: String, multiplier: f64 },
    SubmitUserEdit { name: String, quota: Option<f64>, reset_day: Option<i64>, expire: Option<String>, multiplier: Option<f64> },
    SubmitNode { tag: String, protocol: String, port: u16, server_name: Option<String>, path: Option<String>, port_reuse: bool },
    SubmitNodeEdit { tag: String, port: Option<u16>, server_name: Option<String>, path: Option<String>, port_reuse: Option<bool> },
    DeleteUser(String),
    DeleteNode(String),
    SaveNodePicker { user: String, all: bool, tags: Vec<String> },
    RegenToken(String),
    RevokeToken(String),
}

impl Modal {
    pub fn handle(&mut self, k: KeyEvent) -> ModalAction {
        if matches!(k.code, KeyCode::Esc) { return ModalAction::Close; }
        match self {
            Modal::AddUser(f)  => handle_user(f, k),
            Modal::EditUser(f) => handle_user_edit(f, k),
            Modal::AddNode(f)  => handle_node(f, k),
            Modal::EditNode(f) => handle_node_edit(f, k),
            Modal::ConfirmDeleteUser(name) => match k.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => ModalAction::DeleteUser(name.clone()),
                KeyCode::Char('n') | KeyCode::Char('N') => ModalAction::Close,
                _ => ModalAction::None,
            },
            Modal::ConfirmDeleteNode(tag) => match k.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => ModalAction::DeleteNode(tag.clone()),
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
                KeyCode::Char('v') | KeyCode::Char('V') if *has_token => ModalAction::RevokeToken(name.clone()),
                _ => ModalAction::None,
            },
        }
    }
}

fn handle_user_edit(f: &mut UserEditForm, k: KeyEvent) -> ModalAction {
    const FIELDS: usize = 4;
    f.error = None;
    match k.code {
        KeyCode::Tab | KeyCode::Down => { f.focus = (f.focus + 1) % FIELDS; ModalAction::None }
        KeyCode::BackTab | KeyCode::Up => {
            f.focus = if f.focus == 0 { FIELDS - 1 } else { f.focus - 1 };
            ModalAction::None
        }
        KeyCode::Enter => {
            let q  = if f.quota.trim().is_empty() { None } else {
                match f.quota.trim().parse::<f64>() {
                    Ok(v) => Some(v),
                    Err(_) => { f.error = Some("配额需为数字".into()); return ModalAction::None; }
                }
            };
            let d  = if f.reset_day.trim().is_empty() { None } else {
                match f.reset_day.trim().parse::<i64>() {
                    Ok(v) if v == 0 || v == 32 || (1..=28).contains(&v) => Some(v),
                    _ => { f.error = Some("重置日需 0/1-28/32".into()); return ModalAction::None; }
                }
            };
            let e = if f.expire.trim().is_empty() { None }
                else if f.expire.trim() == "-" { Some(String::new()) }   // 清为永久
                else { Some(f.expire.trim().to_string()) };
            let m = if f.multiplier.trim().is_empty() { None } else {
                match f.multiplier.trim().parse::<f64>() {
                    Ok(v) if v >= 0.0 => Some(v),
                    _ => { f.error = Some("倍率需为大于等于 0 的数字".into()); return ModalAction::None; }
                }
            };
            ModalAction::SubmitUserEdit { name: f.name.clone(), quota: q, reset_day: d, expire: e, multiplier: m }
        }
        KeyCode::Backspace => { user_edit_field(f).pop(); ModalAction::None }
        KeyCode::Char(c) => { user_edit_field(f).push(c); ModalAction::None }
        _ => ModalAction::None,
    }
}

fn user_edit_field(f: &mut UserEditForm) -> &mut String {
    match f.focus { 0 => &mut f.quota, 1 => &mut f.reset_day, 2 => &mut f.expire, _ => &mut f.multiplier }
}

fn handle_node_edit(f: &mut NodeEditForm, k: KeyEvent) -> ModalAction {
    let fields = edit_fields(&f.protocol);
    let n = fields.len().max(1);
    if f.focus >= n { f.focus = n - 1; }
    let focused = fields.get(f.focus).copied();
    f.error = None;
    match k.code {
        KeyCode::Tab | KeyCode::Down => { f.focus = (f.focus + 1) % n; ModalAction::None }
        KeyCode::BackTab | KeyCode::Up => {
            f.focus = if f.focus == 0 { n - 1 } else { f.focus - 1 };
            ModalAction::None
        }
        KeyCode::Left | KeyCode::Right | KeyCode::Char(' ')
            if focused == Some(NodeField::PortReuse) => {
            f.port_reuse = !f.port_reuse;
            ModalAction::None
        }
        KeyCode::Enter => {
            let port = if f.port.trim().is_empty() { None } else {
                match f.port.trim().parse::<u16>() {
                    Ok(v) if v > 0 => Some(v),
                    _ => { f.error = Some("端口需为 1-65535".into()); return ModalAction::None; }
                }
            };
            let sn = if protocol_uses_sni(&f.protocol) && !f.server_name.trim().is_empty() {
                Some(f.server_name.trim().to_string())
            } else { None };
            let pa = if protocol_uses_path(&f.protocol) && !f.path.trim().is_empty() {
                Some(f.path.trim().to_string())
            } else { None };
            // 只有可复用的协议才回传 port_reuse 开关
            let pr = if protocol_supports_port_reuse(&f.protocol) { Some(f.port_reuse) } else { None };
            ModalAction::SubmitNodeEdit { tag: f.tag.clone(), port, server_name: sn, path: pa, port_reuse: pr }
        }
        KeyCode::Backspace if focused != Some(NodeField::PortReuse) => {
            if let Some(s) = node_edit_field_mut(f, focused) { s.pop(); }
            ModalAction::None
        }
        KeyCode::Char(c) if focused != Some(NodeField::PortReuse) => {
            if let Some(s) = node_edit_field_mut(f, focused) { s.push(c); }
            ModalAction::None
        }
        _ => ModalAction::None,
    }
}

fn node_edit_field_mut(f: &mut NodeEditForm, which: Option<NodeField>) -> Option<&mut String> {
    match which? {
        NodeField::Port       => Some(&mut f.port),
        NodeField::ServerName => Some(&mut f.server_name),
        NodeField::Path       => Some(&mut f.path),
        _ => None,
    }
}

fn handle_picker(p: &mut NodePicker, k: KeyEvent) -> ModalAction {
    let len = p.tags.len();
    match k.code {
        KeyCode::Up   | KeyCode::Char('k') => {
            if len > 0 { p.cursor = if p.cursor == 0 { len - 1 } else { p.cursor - 1 }; }
            ModalAction::None
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if len > 0 { p.cursor = (p.cursor + 1) % len; }
            ModalAction::None
        }
        KeyCode::Char(' ') => {
            if let Some(v) = p.checked.get_mut(p.cursor) { *v = !*v; }
            p.all = false;
            ModalAction::None
        }
        KeyCode::Char('a') => {
            // 切换 all
            p.all = !p.all;
            if p.all { for v in p.checked.iter_mut() { *v = false; } }
            ModalAction::None
        }
        KeyCode::Enter => {
            let tags: Vec<String> = if p.all { vec![] }
                else { p.tags.iter().zip(p.checked.iter()).filter_map(|(t, c)| if *c { Some(t.clone()) } else { None }).collect() };
            ModalAction::SaveNodePicker { user: p.user.clone(), all: p.all, tags }
        }
        _ => ModalAction::None,
    }
}

fn handle_user(f: &mut UserForm, k: KeyEvent) -> ModalAction {
    const FIELDS: usize = 5;
    f.error = None;
    match k.code {
        KeyCode::Tab | KeyCode::Down => { f.focus = (f.focus + 1) % FIELDS; ModalAction::None }
        KeyCode::BackTab | KeyCode::Up => {
            f.focus = if f.focus == 0 { FIELDS - 1 } else { f.focus - 1 };
            ModalAction::None
        }
        KeyCode::Enter => {
            let name = f.name.trim().to_string();
            if name.is_empty() { f.error = Some("用户名必填".into()); return ModalAction::None; }
            let quota: f64 = if f.quota.trim().is_empty() { 0.0 }
                else { match f.quota.trim().parse() {
                    Ok(v) => v,
                    Err(_) => { f.error = Some("配额需为数字(GB)，0=不限".into()); return ModalAction::None; }
                }};
            let reset_day: i64 = if f.reset_day.trim().is_empty() { 0 }
                else { match f.reset_day.trim().parse::<i64>() {
                    Ok(v) if v == 0 || v == 32 || (1..=28).contains(&v) => v,
                    _ => { f.error = Some("重置日需 0/1-28/32".into()); return ModalAction::None; }
                }};
            let expire = f.expire.trim().to_string();
            let multiplier: f64 = if f.multiplier.trim().is_empty() { 2.0 }
                else { match f.multiplier.trim().parse() {
                    Ok(v) if v >= 0.0 => v,
                    _ => { f.error = Some("倍率需为大于等于 0 的数字".into()); return ModalAction::None; }
                }};
            ModalAction::SubmitUser { name, quota, reset_day, expire, multiplier }
        }
        KeyCode::Backspace => { user_field(f).pop(); ModalAction::None }
        KeyCode::Char(c) => { user_field(f).push(c); ModalAction::None }
        _ => ModalAction::None,
    }
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
    if f.focus >= n { f.focus = n - 1; }
    let focused = fields[f.focus];
    f.error = None;
    match k.code {
        KeyCode::Tab | KeyCode::Down => { f.focus = (f.focus + 1) % n; ModalAction::None }
        KeyCode::BackTab | KeyCode::Up => {
            f.focus = if f.focus == 0 { n - 1 } else { f.focus - 1 };
            ModalAction::None
        }
        KeyCode::Left if focused == NodeField::Protocol => {
            f.protocol_idx = if f.protocol_idx == 0 { PROTOCOLS.len() - 1 } else { f.protocol_idx - 1 };
            ModalAction::None
        }
        KeyCode::Right if focused == NodeField::Protocol => {
            f.protocol_idx = (f.protocol_idx + 1) % PROTOCOLS.len();
            ModalAction::None
        }
        KeyCode::Left | KeyCode::Right | KeyCode::Char(' ')
            if focused == NodeField::PortReuse => {
            f.port_reuse = !f.port_reuse;
            ModalAction::None
        }
        KeyCode::Enter => {
            let tag = f.tag.trim().to_string();
            if tag.is_empty() { f.error = Some("tag 必填".into()); return ModalAction::None; }
            let port: u16 = match f.port.trim().parse() {
                Ok(v) if v > 0 => v,
                _ => { f.error = Some("端口需为 1-65535".into()); return ModalAction::None; }
            };
            let protocol = PROTOCOLS[f.protocol_idx].to_string();
            // 只在协议实际需要时回传对应字段，避免把 server_name / path 塞进不该有的协议。
            let sn = if protocol_uses_sni(&protocol) && !f.server_name.trim().is_empty() {
                Some(f.server_name.trim().to_string())
            } else { None };
            let path = if protocol_uses_path(&protocol) && !f.path.trim().is_empty() {
                Some(f.path.trim().to_string())
            } else { None };
            let reuse = protocol_supports_port_reuse(&protocol) && f.port_reuse;
            ModalAction::SubmitNode { tag, protocol, port, server_name: sn, path, port_reuse: reuse }
        }
        KeyCode::Backspace if !matches!(focused, NodeField::Protocol | NodeField::PortReuse) => {
            if let Some(s) = node_field_mut(f, focused) { s.pop(); }
            ModalAction::None
        }
        KeyCode::Char(c) if !matches!(focused, NodeField::Protocol | NodeField::PortReuse) => {
            if let Some(s) = node_field_mut(f, focused) { s.push(c); }
            ModalAction::None
        }
        _ => ModalAction::None,
    }
}

fn node_field_mut(f: &mut NodeForm, which: NodeField) -> Option<&mut String> {
    match which {
        NodeField::Tag        => Some(&mut f.tag),
        NodeField::Port       => Some(&mut f.port),
        NodeField::ServerName => Some(&mut f.server_name),
        NodeField::Path       => Some(&mut f.path),
        NodeField::Protocol | NodeField::PortReuse => None,
    }
}

pub fn render(f: &mut Frame, area: Rect, modal: &Modal) {
    let pop = centered(area, 62, 16);
    f.render_widget(Clear, pop);
    match modal {
        Modal::AddUser(form)  => render_user(f, pop, form, "添加用户"),
        Modal::EditUser(form) => render_user_edit(f, pop, form),
        Modal::AddNode(form)  => render_node(f, pop, form),
        Modal::EditNode(form) => render_node_edit(f, pop, form),
        Modal::ConfirmDeleteUser(name) => render_confirm(f, pop, " 确认删除用户 ", name),
        Modal::ConfirmDeleteNode(tag) => render_confirm(f, pop, " 确认删除节点 ", tag),
        Modal::NodePicker(p) => render_picker(f, centered(area, 62, (p.tags.len() as u16 + 8).min(20)), p),
        Modal::SubUrl { name, singbox, mihomo } => {
            // URL 可能很长，modal 宽度用 min(屏宽-4, max(url长度+8, 62))
            let max_len = singbox.len().max(mihomo.len()) as u16 + 8;
            let w = max_len.max(62).min(area.width.saturating_sub(4));
            render_sub_url(f, centered(area, w, 12), name, singbox, mihomo);
        }
        Modal::TokenManage { name, has_token } => {
            render_token_manage(f, centered(area, 62, 10), name, *has_token);
        }
    }
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    let v = Layout::default().direction(Direction::Vertical).constraints([
        Constraint::Length(area.height.saturating_sub(h) / 2),
        Constraint::Length(h),
        Constraint::Min(0),
    ]).split(area);
    Layout::default().direction(Direction::Horizontal).constraints([
        Constraint::Length(area.width.saturating_sub(w) / 2),
        Constraint::Length(w),
        Constraint::Min(0),
    ]).split(v[1])[1]
}

fn render_user(f: &mut Frame, area: Rect, form: &UserForm, title: &str) {
    let labels = ["用户名", "配额 GB (0=不限)", "重置日 (1-28/32/0)", "到期 (YYYY-MM-DD, 例: 2026-12-31)", "流量倍率 (双倍=2.0, 单倍=1.0)"];
    let vals = [&form.name, &form.quota, &form.reset_day, &form.expire, &form.multiplier];
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));
    for (i, (label, val)) in labels.iter().zip(vals).enumerate() {
        let style = if i == form.focus {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else { Style::default().fg(Color::White) };
        let cursor = if i == form.focus { "_" } else { "" };
        lines.push(Line::from(vec![
            Span::styled(format!(" {:<22}", label), Style::default().fg(Color::Yellow)),
            Span::styled(format!(" {}{}  ", val, cursor), style),
        ]));
        lines.push(Line::from(""));
    }
    if let Some(e) = &form.error {
        lines.push(Line::from(Span::styled(format!("  ! {}", e), Style::default().fg(Color::Red))));
    }
    lines.push(Line::from(Span::styled(
        "  Tab/↑↓ 切换   Enter 提交   Esc 取消   (留空使用默认值)",
        Style::default().fg(Color::DarkGray),
    )));
    f.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(format!(" {} ", title))),
        area,
    );
}

fn render_user_edit(f: &mut Frame, area: Rect, form: &UserEditForm) {
    let labels = ["配额 GB (留空不改)", "重置日 (留空不改)", "到期 (留空不改, - 清为永久, 例: 2026-12-31)", "流量倍率 (留空不改, 双倍=2.0)"];
    let vals = [&form.quota, &form.reset_day, &form.expire, &form.multiplier];
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!("  用户: {}  （name 不可改，删掉重建）", form.name),
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
    for (i, (label, val)) in labels.iter().zip(vals).enumerate() {
        let style = if i == form.focus {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else { Style::default().fg(Color::White) };
        let cursor = if i == form.focus { "_" } else { "" };
        lines.push(Line::from(vec![
            Span::styled(format!(" {:<22}", label), Style::default().fg(Color::Yellow)),
            Span::styled(format!(" {}{}  ", val, cursor), style),
        ]));
        lines.push(Line::from(""));
    }
    if let Some(e) = &form.error {
        lines.push(Line::from(Span::styled(format!("  ! {}", e), Style::default().fg(Color::Red))));
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

fn render_node(f: &mut Frame, area: Rect, form: &NodeForm) {
    let protocol = PROTOCOLS[form.protocol_idx];
    let fields = add_fields(protocol);
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));
    for (i, field) in fields.iter().enumerate() {
        let (label, val): (&str, String) = match field {
            NodeField::Tag        => ("Tag *必填",             form.tag.clone()),
            NodeField::Protocol   => ("协议 (←/→ 切换)",       format!("◀ {} ▶", protocol)),
            NodeField::Port       => ("端口 *必填 (默认 443)", form.port.clone()),
            NodeField::ServerName => ("server_name (SNI)",     form.server_name.clone()),
            NodeField::Path       => ("path (留空=默认)",      form.path.clone()),
            NodeField::PortReuse  => (
                "端口复用 (Space/←→ 切换)",
                format!("◀ {} ▶", if form.port_reuse { "开" } else { "关" }),
            ),
        };
        let style = if i == form.focus {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else { Style::default().fg(Color::White) };
        let cursor = if i == form.focus && !matches!(*field, NodeField::Protocol | NodeField::PortReuse) { "_" } else { "" };
        lines.push(Line::from(vec![
            Span::styled(format!(" {:<24}", label), Style::default().fg(Color::Yellow)),
            Span::styled(format!(" {}{}  ", val, cursor), style),
        ]));
        lines.push(Line::from(""));
    }
    if let Some(e) = &form.error {
        lines.push(Line::from(Span::styled(format!("  ! {}", e), Style::default().fg(Color::Red))));
    }
    let hint = match protocol {
        "vless-reality" => "  reality: private_key/short_id 自动生成；server_name 同时作为 handshake 目标",
        "vless-ws" | "vmess-ws" => "  ws: 后端不启 TLS，建议前挂 nginx/caddy 终结 TLS",
        "shadowsocks" => "  shadowsocks-2022：密钥自动生成，无 SNI / path 字段",
        "hysteria2" => "  hysteria2: 无 server_name / path；证书 CN=tag，客户端订阅走 insecure=1",
        "trojan" | "tuic" | "anytls" => "  自签证书 CN=server_name；客户端订阅自动带 allowInsecure/insecure",
        _ => "",
    };
    if !hint.is_empty() {
        lines.push(Line::from(Span::styled(hint, Style::default().fg(Color::DarkGray))));
    }
    if protocol_supports_port_reuse(protocol) {
        lines.push(Line::from(Span::styled(
            "  端口复用开启：listen→127.0.0.1，订阅端口写 443；需手动配 nginx stream SNI 分流",
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines.push(Line::from(Span::styled(
        "  Tab/↑↓ 切换   ←/→ 选协议   Enter 提交   Esc 取消",
        Style::default().fg(Color::DarkGray),
    )));
    f.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" 添加节点 ")),
        area,
    );
}

fn render_node_edit(f: &mut Frame, area: Rect, form: &NodeEditForm) {
    let fields = edit_fields(&form.protocol);
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!("  Tag: {}   协议: {}   （tag/协议不可改，删掉重建）", form.tag, form.protocol),
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
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
            NodeField::Port       => ("端口 (留空不改)",        form.port.clone()),
            NodeField::ServerName => ("server_name (留空不改)", form.server_name.clone()),
            NodeField::Path       => ("path (留空不改)",        form.path.clone()),
            NodeField::PortReuse  => (
                "端口复用 (Space/←→ 切换)",
                format!("◀ {} ▶", if form.port_reuse { "开" } else { "关" }),
            ),
            _ => continue,
        };
        let style = if i == form.focus {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else { Style::default().fg(Color::White) };
        let cursor = if i == form.focus && *field != NodeField::PortReuse { "_" } else { "" };
        lines.push(Line::from(vec![
            Span::styled(format!(" {:<28}", label), Style::default().fg(Color::Yellow)),
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
        lines.push(Line::from(Span::styled(format!("  ! {}", e), Style::default().fg(Color::Red))));
    }
    lines.push(Line::from(Span::styled(
        "  Tab/↑↓ 切换   Enter 保存   Esc 取消",
        Style::default().fg(Color::DarkGray),
    )));
    f.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" 编辑节点 ")),
        area,
    );
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
        Paragraph::new(text).alignment(Alignment::Left)
            .block(Block::default().borders(Borders::ALL).title(title)),
        area,
    );
}

fn render_token_manage(f: &mut Frame, area: Rect, name: &str, has_token: bool) {
    f.render_widget(Clear, area);
    let mut lines: Vec<Line> = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  用户: {}   当前: {}", name, if has_token { "● 订阅已开启" } else { "○ 订阅已关闭" }),
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
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
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled("  sing-box / v2rayN:", Style::default().fg(Color::Cyan))),
        Line::from(Span::styled(format!("    {}", singbox), Style::default().fg(Color::White))),
        Line::from(""),
        Line::from(Span::styled("  mihomo / Clash-meta:", Style::default().fg(Color::Cyan))),
        Line::from(Span::styled(format!("    {}", mihomo), Style::default().fg(Color::White))),
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
        format!("  用户: {}   当前: {}", p.user, if p.all { "全部节点" } else { "按列表" }),
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
    if p.tags.is_empty() {
        lines.push(Line::from(Span::styled("  （没有节点，先去节点页按 [a] 添加）", Style::default().fg(Color::DarkGray))));
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
            } else { Style::default().fg(Color::White) };
            lines.push(Line::from(Span::styled(format!("  {} {}", mark, t), style)));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  ↑↓/jk 选择   Space 勾选   a 切换全部   Enter 保存   Esc 取消",
        Style::default().fg(Color::DarkGray),
    )));
    f.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" 分配可用节点 ")),
        area,
    );
}
