use ratatui::{Frame,layout::{Constraint,Direction,Layout,Rect},
    style::{Color,Modifier,Style},text::{Line,Span},
    widgets::{Block,Borders,Cell,Paragraph,Row,Table,Wrap}};
use crate::{tui::app::AppState, model::user::User};

pub fn render(f: &mut Frame, area: Rect, s: &AppState) {
    let c = Layout::default().direction(Direction::Vertical)
        .constraints([Constraint::Min(0),Constraint::Length(4)]).split(area);
    let hdr = Row::new(["用户名","状态","上行","下行","总量","配额","重置","到期"]
        .map(|h| Cell::from(h).style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)))).height(1);
    let rows: Vec<Row> = s.users.iter().enumerate().map(|(i,u)| {
        let sel = i==s.user_table.selected;
        let bs  = if sel {Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD)} else {Style::default()};
        let sc  = if !u.enabled {Color::Red} else if u.is_over_quota()||u.is_expired() {Color::Yellow} else {Color::Green};
        let quota = if u.quota_gb<=0.0 {"不限".into()} else {format!("{:.0}G({:.0}%)",u.quota_gb,u.quota_used_percent())};
        let reset = match u.reset_day {0=>"─".into(),32=>"月末".into(),d=>format!("{}日",d)};
        let exp   = if u.expire_at.is_empty() {"永久".into()} else {u.expire_at.clone()};
        Row::new(vec![
            Cell::from(u.name.clone()),
            Cell::from(if u.enabled{"● 启用"}else{"○ 禁用"}).style(Style::default().fg(sc)),
            Cell::from(User::format_bytes(u.used_up_bytes)),
            Cell::from(User::format_bytes(u.used_down_bytes)),
            Cell::from(User::format_bytes(u.used_total_bytes())),
            Cell::from(quota),Cell::from(reset),Cell::from(exp),
        ]).style(bs)
    }).collect();
    f.render_widget(Table::new(rows,[
        Constraint::Length(14),Constraint::Length(8),Constraint::Length(10),Constraint::Length(10),
        Constraint::Length(10),Constraint::Length(14),Constraint::Length(8),Constraint::Length(12),
    ]).header(hdr).block(Block::default().borders(Borders::ALL).title(" 用户列表 ")), c[0]);
    let sel = s.users.get(s.user_table.selected)
        .map(|u| format!("  选中: {}  总: {}", u.name, User::format_bytes(u.used_total_bytes())))
        .unwrap_or("  (无用户)".into());
    f.render_widget(Paragraph::new(vec![
        Line::from(sel),
        Line::from(Span::styled("  [a]添加  [d]删除  [t]启/禁  [r]重置流量  [s]导出订阅  [n]分配节点  [R]刷新",Style::default().fg(Color::DarkGray))),
    ]).block(Block::default().borders(Borders::ALL).title(" 操作 ")).wrap(Wrap{trim:true}), c[1]);
}
