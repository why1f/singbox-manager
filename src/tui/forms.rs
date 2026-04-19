use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

pub const PROTOCOLS: [&str; 8] = [
    "vless-reality", "vless-ws", "vmess-ws", "trojan",
    "shadowsocks", "hysteria2", "tuic", "anytls",
];

#[derive(Default)]
pub struct UserForm {
    pub name: String,
    pub quota: String,
    pub reset_day: String,
    pub expire: String,
    pub focus: usize,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct NodeForm {
    pub tag: String,
    pub protocol_idx: usize,
    pub port: String,
    pub server_name: String,
    pub path: String,
    pub focus: usize,
    pub error: Option<String>,
}

pub enum Modal {
    AddUser(UserForm),
    AddNode(NodeForm),
    ConfirmDeleteUser(String),
    ConfirmDeleteNode(String),
}

pub enum ModalAction {
    None,
    Close,
    SubmitUser { name: String, quota: f64, reset_day: i64, expire: String },
    SubmitNode { tag: String, protocol: String, port: u16, server_name: Option<String>, path: Option<String> },
    DeleteUser(String),
    DeleteNode(String),
}

impl Modal {
    pub fn handle(&mut self, k: KeyEvent) -> ModalAction {
        if matches!(k.code, KeyCode::Esc) { return ModalAction::Close; }
        match self {
            Modal::AddUser(f) => handle_user(f, k),
            Modal::AddNode(f) => handle_node(f, k),
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
        }
    }
}

fn handle_user(f: &mut UserForm, k: KeyEvent) -> ModalAction {
    const FIELDS: usize = 4;
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
            ModalAction::SubmitUser { name, quota, reset_day, expire }
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
        _ => &mut f.expire,
    }
}

fn handle_node(f: &mut NodeForm, k: KeyEvent) -> ModalAction {
    const FIELDS: usize = 5;
    f.error = None;
    match k.code {
        KeyCode::Tab | KeyCode::Down => { f.focus = (f.focus + 1) % FIELDS; ModalAction::None }
        KeyCode::BackTab | KeyCode::Up => {
            f.focus = if f.focus == 0 { FIELDS - 1 } else { f.focus - 1 };
            ModalAction::None
        }
        KeyCode::Left if f.focus == 1 => {
            f.protocol_idx = if f.protocol_idx == 0 { PROTOCOLS.len() - 1 } else { f.protocol_idx - 1 };
            ModalAction::None
        }
        KeyCode::Right if f.focus == 1 => {
            f.protocol_idx = (f.protocol_idx + 1) % PROTOCOLS.len();
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
            let sn = if f.server_name.trim().is_empty() { None } else { Some(f.server_name.trim().to_string()) };
            let path = if f.path.trim().is_empty() { None } else { Some(f.path.trim().to_string()) };
            ModalAction::SubmitNode { tag, protocol, port, server_name: sn, path }
        }
        KeyCode::Backspace if f.focus != 1 => { node_field(f).pop(); ModalAction::None }
        KeyCode::Char(c) if f.focus != 1 => { node_field(f).push(c); ModalAction::None }
        _ => ModalAction::None,
    }
}

fn node_field(f: &mut NodeForm) -> &mut String {
    match f.focus {
        0 => &mut f.tag,
        2 => &mut f.port,
        3 => &mut f.server_name,
        _ => &mut f.path,
    }
}

pub fn render(f: &mut Frame, area: Rect, modal: &Modal) {
    let pop = centered(area, 62, 16);
    f.render_widget(Clear, pop);
    match modal {
        Modal::AddUser(form) => render_user(f, pop, form),
        Modal::AddNode(form) => render_node(f, pop, form),
        Modal::ConfirmDeleteUser(name) => render_confirm(f, pop, " 确认删除用户 ", name),
        Modal::ConfirmDeleteNode(tag) => render_confirm(f, pop, " 确认删除节点 ", tag),
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

fn render_user(f: &mut Frame, area: Rect, form: &UserForm) {
    let labels = ["用户名", "配额 GB (0=不限)", "重置日 (1-28/32/0)", "到期 (YYYY-MM-DD)"];
    let vals = [&form.name, &form.quota, &form.reset_day, &form.expire];
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
        "  Tab/↑↓ 切换   Enter 提交   Esc 取消",
        Style::default().fg(Color::DarkGray),
    )));
    f.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" 添加用户 ")),
        area,
    );
}

fn render_node(f: &mut Frame, area: Rect, form: &NodeForm) {
    let labels = ["Tag", "协议", "端口", "server_name (可选)", "path (可选)"];
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));
    for (i, label) in labels.iter().enumerate() {
        let val: String = match i {
            0 => form.tag.clone(),
            1 => format!("◀ {} ▶", PROTOCOLS[form.protocol_idx]),
            2 => form.port.clone(),
            3 => form.server_name.clone(),
            _ => form.path.clone(),
        };
        let style = if i == form.focus {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else { Style::default().fg(Color::White) };
        let cursor = if i == form.focus && i != 1 { "_" } else { "" };
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
        "  Tab/↑↓ 切换   ←/→ 选协议   Enter 提交   Esc 取消",
        Style::default().fg(Color::DarkGray),
    )));
    f.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" 添加节点 ")),
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
