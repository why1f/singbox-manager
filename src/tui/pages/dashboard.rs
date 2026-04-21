use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols::{self, Marker},
    text::{Line, Span},
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType, Gauge, Paragraph, Wrap},
    Frame,
};
use crate::{model::user::User, tui::app::AppState};

pub fn render(f: &mut Frame, area: Rect, s: &AppState) {
    let c = Layout::default().direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // sing-box 状态
            Constraint::Length(3),  // CPU gauge
            Constraint::Length(8),  // 网速双曲线（braille）
            Constraint::Min(6),     // 用户摘要
            Constraint::Length(6),  // 节点摘要
        ]).split(area);

    render_status(f, c[0], s);
    render_cpu(f, c[1], s);
    render_net(f, c[2], s);
    render_summary(f, c[3], s);
    render_nodes(f, c[4], s);
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
            Span::raw("   "),
            Span::styled("[U] 升级", Style::default().fg(Color::Cyan)),
        ]))
        .block(Block::default().borders(Borders::ALL).title(" sing-box 状态 ")),
        area,
    );
}

fn render_cpu(f: &mut Frame, area: Rect, s: &AppState) {
    let cur = s.cpu_history.last().copied().unwrap_or(0);
    let color = if cur >= 90 { Color::Red } else if cur >= 60 { Color::Yellow } else { Color::Green };
    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title(format!(" CPU  {}% ", cur)))
        .gauge_style(Style::default().fg(color))
        .ratio((cur.min(100) as f64) / 100.0)
        .label("");
    f.render_widget(gauge, area);
}

fn render_net(f: &mut Frame, area: Rect, s: &AppState) {
    let cc = Layout::default().direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let rx_cur = s.net_rx_history.last().copied().unwrap_or(0);
    let tx_cur = s.net_tx_history.last().copied().unwrap_or(0);
    render_net_chart(f, cc[0], &s.net_rx_history, Color::Cyan,    format!(" ↓ 下行  {}/s ", fmt_bytes(rx_cur)));
    render_net_chart(f, cc[1], &s.net_tx_history, Color::Magenta, format!(" ↑ 上行  {}/s ", fmt_bytes(tx_cur)));
}

fn render_net_chart(f: &mut Frame, area: Rect, hist: &[u64], color: Color, title: String) {
    let data: Vec<(f64, f64)> = hist.iter().enumerate()
        .map(|(i, v)| (i as f64, *v as f64))
        .collect();
    // Y 轴上限用历史最大值 * 1.2，避免 spike 顶到边；保底 1KB 尺度防止空数据
    let y_max = data.iter().map(|(_, v)| *v).fold(0.0_f64, f64::max).max(1024.0) * 1.2;
    let x_max = (hist.len().max(1) - 1) as f64;

    let datasets = vec![
        Dataset::default()
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(color))
            .data(&data),
    ];
    let chart = Chart::new(datasets)
        .block(Block::default().borders(Borders::ALL).title(title))
        .x_axis(Axis::default().bounds([0.0, x_max.max(1.0)]))
        .y_axis(Axis::default().bounds([0.0, y_max]));
    f.render_widget(chart, area);
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

    let bar_width = (area.width.saturating_sub(50)).clamp(10, 30) as usize;

    let mut lines: Vec<Line> = vec![
        Line::from(vec![
            Span::raw("  用户 "),
            Span::styled(format!("{} ", total), Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(format!("启用:{} ", en),    Style::default().fg(Color::Green)),
            Span::styled(format!("超额:{} ", over),  Style::default().fg(Color::Red)),
            Span::styled(format!("到期:{} ", exp),   Style::default().fg(Color::Yellow)),
            Span::styled(format!("  累计 ↑{} ↓{}", User::format_bytes(up_b), User::format_bytes(dn_b)),
                Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(""),
    ];
    if top.is_empty() {
        lines.push(Line::from(Span::styled(
            "  （无用户，进用户页按 [a] 添加）",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "  用量 Top 5",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )));
        for u in &top {
            let total = u.used_total_bytes();
            let pct = u.quota_used_percent() as u8;
            let c = if !u.enabled { Color::Red }
                else if u.is_over_quota() || u.is_expired() { Color::Yellow }
                else if u.quota_gb <= 0.0 { Color::DarkGray }
                else { Color::Green };
            let bar = progress_bar(pct, bar_width, u.quota_gb > 0.0);
            let quota = if u.quota_gb <= 0.0 { "不限".to_string() } else { format!("{:.0}G", u.quota_gb) };
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(format!("{:<14}", u.name), Style::default().fg(c)),
                Span::styled(format!(" {:<10}", User::format_bytes(total)), Style::default().fg(Color::White)),
                Span::styled(format!(" {:<6}", quota),                       Style::default().fg(Color::DarkGray)),
                Span::styled(bar,                                            Style::default().fg(c)),
                Span::styled(format!(" {:>5.1}%", u.quota_used_percent()),   Style::default().fg(c)),
            ]));
        }
    }

    f.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(" 用户摘要 "))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_nodes(f: &mut Frame, area: Rect, s: &AppState) {
    let mut lines: Vec<Line> = Vec::new();
    if s.nodes.is_empty() {
        lines.push(Line::from(Span::styled(
            "  （无节点，进节点页按 [a] 添加）",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        lines.push(Line::from(vec![
            Span::styled(format!("  共 {} 个节点  ", s.nodes.len()),
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled("(Tab 或按 3 进入节点页)", Style::default().fg(Color::DarkGray)),
        ]));
        for n in s.nodes.iter().take(6) {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(format!("{:<18}", n.tag),      Style::default().fg(Color::Cyan)),
                Span::styled(format!("{:<14}", n.protocol), Style::default().fg(Color::White)),
                Span::styled(format!(":{:<6}", n.listen_port), Style::default().fg(Color::DarkGray)),
                Span::styled(format!(" 用户 {}", n.user_count), Style::default().fg(Color::Green)),
            ]));
        }
        if s.nodes.len() > 6 {
            lines.push(Line::from(Span::styled(
                format!("  … 还有 {} 个，见节点页", s.nodes.len() - 6),
                Style::default().fg(Color::DarkGray),
            )));
        }
    }
    f.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(" 节点摘要 "))
            .wrap(Wrap { trim: false }),
        area,
    );
}

/// 返回形如 `██████░░░░` 的进度条字符串；quota<=0 时返回空白占位
pub fn progress_bar(pct: u8, width: usize, has_quota: bool) -> String {
    if width < 2 || !has_quota { return " ".repeat(width); }
    let p = pct.min(100) as usize;
    let filled = (p * width) / 100;
    let empty = width - filled;
    let mut s = String::with_capacity(width * 3);
    s.push_str(&"█".repeat(filled));
    s.push_str(&"░".repeat(empty));
    s
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

// 避免 symbols 未用（Gauge 仍需要隐式）
#[allow(dead_code)] fn _symbols_link() { let _ = symbols::DOT; }
