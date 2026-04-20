use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph, Sparkline, Wrap},
    Frame,
};
use crate::{model::user::User, tui::app::AppState};

pub fn render(f: &mut Frame, area: Rect, s: &AppState) {
    let c = Layout::default().direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // sing-box 状态
            Constraint::Length(3),  // CPU gauge
            Constraint::Length(6),  // 网速双曲线
            Constraint::Min(0),     // 用户摘要
        ]).split(area);

    render_status(f, c[0], s);
    render_cpu(f, c[1], s);
    render_net(f, c[2], s);
    render_summary(f, c[3], s);
}

fn render_status(f: &mut Frame, area: Rect, s: &AppState) {
    let (sc, st) = match s.singbox_running {
        Some(true)  => (Color::Green,  "● 运行中"),
        Some(false) => (Color::Red,    "○ 已停止"),
        None        => (Color::Yellow, "○ 未检测"),
    };
    let (gc, gt) = if s.grpc_connected { (Color::Green, "gRPC ✓") } else { (Color::Yellow, "gRPC ✗") };
    let sync = s.last_sync_time.map(|t| format!("同步:{}", t.format("%H:%M:%S"))).unwrap_or_else(|| "未同步".into());
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("  "),
            Span::styled(st, Style::default().fg(sc).add_modifier(Modifier::BOLD)),
            Span::raw("   "),
            Span::styled(gt, Style::default().fg(gc)),
            Span::raw("   "),
            Span::styled(sync, Style::default().fg(Color::DarkGray)),
            Span::raw(format!("   运行:{}s", s.uptime_secs)),
        ]))
        .block(Block::default().borders(Borders::ALL).title(" sing-box 状态 ")),
        area,
    );
}

fn render_cpu(f: &mut Frame, area: Rect, s: &AppState) {
    let cur = s.cpu_history.last().copied().unwrap_or(0);
    let color = if cur >= 90 { Color::Red } else if cur >= 60 { Color::Yellow } else { Color::Green };
    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title(" CPU "))
        .gauge_style(Style::default().fg(color))
        .percent(cur.min(100) as u16)
        .label(format!(" {}% ", cur));
    f.render_widget(gauge, area);
}

fn render_net(f: &mut Frame, area: Rect, s: &AppState) {
    let cc = Layout::default().direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let rx_cur = s.net_rx_history.last().copied().unwrap_or(0);
    let tx_cur = s.net_tx_history.last().copied().unwrap_or(0);
    let rx: Vec<u64> = s.net_rx_history.to_vec();
    let tx: Vec<u64> = s.net_tx_history.to_vec();

    f.render_widget(
        Sparkline::default()
            .block(Block::default().borders(Borders::ALL).title(format!(" ↓ 下行  {}/s ", fmt_bytes(rx_cur))))
            .data(&rx).style(Style::default().fg(Color::Cyan))
            .bar_set(symbols::bar::NINE_LEVELS),
        cc[0],
    );
    f.render_widget(
        Sparkline::default()
            .block(Block::default().borders(Borders::ALL).title(format!(" ↑ 上行  {}/s ", fmt_bytes(tx_cur))))
            .data(&tx).style(Style::default().fg(Color::Magenta))
            .bar_set(symbols::bar::NINE_LEVELS),
        cc[1],
    );
}

fn render_summary(f: &mut Frame, area: Rect, s: &AppState) {
    let total = s.users.len();
    let en = s.users.iter().filter(|u| u.enabled).count();
    let over = s.users.iter().filter(|u| u.is_over_quota()).count();
    let exp = s.users.iter().filter(|u| u.is_expired()).count();
    let up_b: i64 = s.users.iter().map(|u| u.used_up_bytes).sum();
    let dn_b: i64 = s.users.iter().map(|u| u.used_down_bytes).sum();

    let mut top: Vec<&User> = s.users.iter().collect();
    top.sort_by_key(|u| -(u.used_total_bytes()));
    let top: Vec<&User> = top.into_iter().take(5).collect();

    let mut lines: Vec<Line> = vec![
        Line::from(vec![
            Span::raw(format!("  用户:{} ", total)),
            Span::styled(format!("启用:{} ", en),    Style::default().fg(Color::Green)),
            Span::styled(format!("超额:{} ", over),  Style::default().fg(Color::Red)),
            Span::styled(format!("到期:{} ", exp),   Style::default().fg(Color::Yellow)),
        ]),
        Line::from(format!("  累计 ↑{} ↓{}", User::format_bytes(up_b), User::format_bytes(dn_b))),
        Line::from(""),
    ];
    if top.is_empty() {
        lines.push(Line::from(Span::styled(
            "  （无用户，进用户页按 [a] 添加）",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "  用量前 5",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )));
        for u in &top {
            let total = u.used_total_bytes();
            let pct = u.quota_used_percent();
            let quota = if u.quota_gb <= 0.0 { "不限".to_string() } else { format!("{:.0}G", u.quota_gb) };
            let c = if !u.enabled { Color::Red }
                else if u.is_over_quota() || u.is_expired() { Color::Yellow }
                else { Color::Green };
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(format!("{:<14}", u.name), Style::default().fg(c)),
                Span::raw(format!(" {:<10}", User::format_bytes(total))),
                Span::raw(format!(" {:<6}", quota)),
                Span::raw(format!(" {:>5.1}%", pct)),
            ]));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  [Tab/1-5]切换  [q]退出  [↑↓/jk]选择  [a]添加  [E]编辑  [d]删除  [t]启禁  [r]重置  [R]刷新",
        Style::default().fg(Color::DarkGray),
    )));

    f.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(" 用户摘要 "))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn fmt_bytes(n: u64) -> String {
    const TB: u64 = 1_099_511_627_776;
    const GB: u64 = 1_073_741_824;
    const MB: u64 = 1_048_576;
    const KB: u64 = 1_024;
    match n {
        b if b >= TB => format!("{:.2} TB", b as f64 / TB as f64),
        b if b >= GB => format!("{:.2} GB", b as f64 / GB as f64),
        b if b >= MB => format!("{:.2} MB", b as f64 / MB as f64),
        b if b >= KB => format!("{:.2} KB", b as f64 / KB as f64),
        b            => format!("{} B", b),
    }
}
