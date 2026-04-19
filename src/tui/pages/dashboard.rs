use ratatui::{Frame,layout::{Constraint,Direction,Layout,Rect},
    style::{Color,Modifier,Style},symbols,text::{Line,Span},
    widgets::{Block,Borders,Paragraph,Sparkline,Wrap}};
use crate::{tui::app::AppState, model::user::User};

pub fn render(f: &mut Frame, area: Rect, s: &AppState) {
    let c = Layout::default().direction(Direction::Vertical)
        .constraints([Constraint::Length(3),Constraint::Length(7),Constraint::Min(0)]).split(area);
    // 状态栏
    let (sc,st) = match s.singbox_running {
        Some(true)  => (Color::Green,"● 运行中"),
        Some(false) => (Color::Red,"○ 已停止"),
        None        => (Color::Yellow,"○ 未检测"),
    };
    let (gc,gt) = if s.grpc_connected  {(Color::Green,"gRPC ✓")}  else {(Color::Yellow,"gRPC ✗")};
    let sync = s.last_sync_time.map(|t|format!("同步:{}",t.format("%H:%M:%S"))).unwrap_or("未同步".into());
    f.render_widget(Paragraph::new(Line::from(vec![
        Span::raw("  "),
        Span::styled(st,Style::default().fg(sc).add_modifier(Modifier::BOLD)),
        Span::raw("   "),Span::styled(gt,Style::default().fg(gc)),
        Span::raw("   "),Span::styled(sync,Style::default().fg(Color::DarkGray)),
        Span::raw(format!("   运行:{}s",s.uptime_secs)),
    ])).block(Block::default().borders(Borders::ALL).title(" sing-box 状态 ")), c[0]);
    // 流量图
    let cc = Layout::default().direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50),Constraint::Percentage(50)]).split(c[1]);
    let up:   Vec<u64> = s.traffic_history.iter().map(|(u,_)| (*u).max(0) as u64).collect();
    let down: Vec<u64> = s.traffic_history.iter().map(|(_,d)| (*d).max(0) as u64).collect();
    f.render_widget(Sparkline::default().block(Block::default().borders(Borders::ALL).title(" ↑ 上行 "))
        .data(&up).style(Style::default().fg(Color::Cyan)).bar_set(symbols::bar::NINE_LEVELS), cc[0]);
    f.render_widget(Sparkline::default().block(Block::default().borders(Borders::ALL).title(" ↓ 下行 "))
        .data(&down).style(Style::default().fg(Color::Magenta)).bar_set(symbols::bar::NINE_LEVELS), cc[1]);
    // 摘要
    let total=s.users.len(); let en=s.users.iter().filter(|u|u.enabled).count();
    let over=s.users.iter().filter(|u|u.is_over_quota()).count();
    let exp=s.users.iter().filter(|u|u.is_expired()).count();
    let up_b:i64=s.users.iter().map(|u|u.used_up_bytes).sum();
    let dn_b:i64=s.users.iter().map(|u|u.used_down_bytes).sum();
    f.render_widget(Paragraph::new(vec![
        Line::from(vec![Span::raw(format!("  用户:{} ",total)),
            Span::styled(format!("启用:{} ",en),Style::default().fg(Color::Green)),
            Span::styled(format!("超额:{} ",over),Style::default().fg(Color::Red)),
            Span::styled(format!("到期:{} ",exp),Style::default().fg(Color::Yellow))]),
        Line::from(""),
        Line::from(format!("  累计 ↑{} ↓{}",User::format_bytes(up_b),User::format_bytes(dn_b))),
        Line::from(""),
        Line::from(Span::styled("  [Tab/1-4]切换  [q]退出  [↑↓/jk]选择  [a]添加  [d]删除  [t]启禁  [r]重置  [R]刷新  [c]检查配置",Style::default().fg(Color::DarkGray))),
    ]).block(Block::default().borders(Borders::ALL).title(" 用户摘要 ")).wrap(Wrap{trim:true}), c[2]);
}
