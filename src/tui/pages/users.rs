use ratatui::{Frame,layout::{Constraint,Direction,Layout,Rect},
    style::{Color,Modifier,Style},text::{Line,Span},
    widgets::{Block,Borders,Cell,Paragraph,Row,Table,Wrap}};
use crate::{tui::app::AppState, model::user::User};

pub fn render(f: &mut Frame, area: Rect, s: &AppState) {
    let c = Layout::default().direction(Direction::Vertical)
        .constraints([Constraint::Min(0),Constraint::Length(5)]).split(area);
    let hdr = Row::new(["用户名","状态","上行","下行","用量","配额/进度","重置","到期","计费","节点"]
        .map(|h| Cell::from(h).style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)))).height(1);
    let rows: Vec<Row> = s.users.iter().enumerate().map(|(i,u)| {
        let sel = i==s.user_table.selected;
        let bs  = if sel {Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD)} else {Style::default()};
        let sc  = if !u.enabled {Color::Red} else if u.is_over_quota()||u.is_expired() {Color::Yellow} else {Color::Green};
        let has_quota = u.quota_gb > 0.0;
        let quota_str = if has_quota {
            let bar = super::super::pages::dashboard::progress_bar(u.quota_used_percent() as u8, 10, true);
            let quota_label = format!("{:.0}G", u.quota_gb);
            format!("{:<5} {} {:>3.0}%", quota_label, bar, u.quota_used_percent())
        } else {
            "不限".into()
        };
        let reset = match u.reset_day {0=>"─".into(),32=>"月末".into(),d=>format!("{}日",d)};
        let exp   = if u.expire_at.is_empty() {"永久".into()} else {u.expire_at.clone()};
        let nodes_cell = if u.allow_all_nodes {
            "全部".to_string()
        } else {
            let tags = u.allowed_tags();
            if tags.is_empty() { "─".into() } else { format!("{}个", tags.len()) }
        };
        let billing = if (u.traffic_multiplier - 2.0).abs() < 0.01 { "双向".to_string() }
            else if (u.traffic_multiplier - 1.0).abs() < 0.01 { "单向".to_string() }
            else { format!("{:.1}x", u.traffic_multiplier) };
        Row::new(vec![
            Cell::from(u.name.clone()),
            Cell::from(if u.enabled{"● 启用"}else{"○ 禁用"}).style(Style::default().fg(sc)),
            Cell::from(User::format_bytes(u.used_up_bytes)),
            Cell::from(User::format_bytes(u.used_down_bytes)),
            Cell::from(User::format_bytes(u.used_total_bytes())),
            Cell::from(quota_str).style(Style::default().fg(sc)),
            Cell::from(reset),Cell::from(exp),
            Cell::from(billing),
            Cell::from(nodes_cell),
        ]).style(bs)
    }).collect();
    f.render_widget(Table::new(rows,[
        Constraint::Length(14),Constraint::Length(8),Constraint::Length(10),Constraint::Length(10),
        Constraint::Length(10),Constraint::Length(24),Constraint::Length(8),Constraint::Length(12),
        Constraint::Length(8),Constraint::Length(8),
    ]).header(hdr).block(Block::default().borders(Borders::ALL).title(" 用户列表 ")), c[0]);

    let sel_text = s.users.get(s.user_table.selected)
        .map(|u| {
            let tags = if u.allow_all_nodes { "全部".to_string() }
                else { let t = u.allowed_tags(); if t.is_empty() { "无".into() } else { t.join(", ") } };
            let sub_url = if u.sub_token.is_empty() {
                "(无 token)".to_string()
            } else if let Some(base) = &s.sub_public_base {
                format!("{}/sub/{}", base.trim_end_matches('/'), u.sub_token)
            } else {
                format!("token={} (未配置 public_base)", u.sub_token)
            };
            format!("  选中: {}  节点: {}\n  订阅: {}", u.name, tags, sub_url)
        })
        .unwrap_or("  (无用户)".into());
    f.render_widget(Paragraph::new(vec![
        Line::from(sel_text),
        Line::from(Span::styled(
            "  [a]添加  [E]编辑  [d]删除  [t]启/禁  [r]重置  [T]token  [u]复制URL  [s]打印  [n]分配节点  [R]刷新",
            Style::default().fg(Color::DarkGray))),
    ]).block(Block::default().borders(Borders::ALL).title(" 操作 ")).wrap(Wrap{trim:true}), c[1]);
}
