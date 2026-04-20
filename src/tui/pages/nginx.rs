use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};
use crate::tui::app::AppState;

pub fn render(f: &mut Frame, area: Rect, s: &AppState) {
    let c = Layout::default().direction(Direction::Vertical)
        .constraints([Constraint::Length(10), Constraint::Min(0)])
        .split(area);

    let status_lines = match &s.nginx {
        None => vec![
            Line::from(""),
            Line::from(Span::styled("  加载中……按 R 刷新", Style::default().fg(Color::DarkGray))),
        ],
        Some(k) => {
            let installed = if k.installed {
                Span::styled("● 已安装", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
            } else {
                Span::styled("○ 未安装", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
            };
            let running = match k.running {
                Some(true)  => Span::styled("● 运行中", Style::default().fg(Color::Green)),
                Some(false) => Span::styled("○ 已停止", Style::default().fg(Color::Yellow)),
                None        => Span::styled("? 未知",   Style::default().fg(Color::DarkGray)),
            };
            let enabled = if k.enabled {
                Span::styled("✓ 已开机自启", Style::default().fg(Color::Green))
            } else {
                Span::styled("✗ 未开机自启", Style::default().fg(Color::Yellow))
            };
            let conf = if k.conf_exists {
                Span::styled("✓ sb-manager 反代已生成", Style::default().fg(Color::Green))
            } else {
                Span::styled("✗ sb-manager 反代未生成 (按 [g])", Style::default().fg(Color::Yellow))
            };
            vec![
                Line::from(""),
                Line::from(vec![Span::raw("  状态: "), installed, Span::raw("    "), running, Span::raw("    "), enabled]),
                Line::from(format!("  版本: {}", k.version.as_deref().unwrap_or("—"))),
                Line::from(""),
                Line::from(vec![Span::raw("  "), conf]),
                Line::from(format!("  订阅 URL: {}/sub/<token>", s.nginx_public_base.as_deref().unwrap_or("(未配置 public_base)"))),
                Line::from(""),
                Line::from(Span::styled(
                    match s.nginx_busy {
                        Some(op) => format!("  正在执行: {}", op),
                        None => "  空闲".into(),
                    },
                    Style::default().fg(if s.nginx_busy.is_some() { Color::Cyan } else { Color::DarkGray }),
                )),
            ]
        }
    };
    f.render_widget(
        Paragraph::new(status_lines).block(Block::default().borders(Borders::ALL).title(" nginx ")),
        c[0],
    );

    let help = vec![
        Line::from(""),
        Line::from(Span::styled("  操作快捷键", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from("  [i]  包管理器安装 nginx"),
        Line::from("  [g]  生成 sb-manager 反代配置 (需要 [subscription].public_base)"),
        Line::from("  [t]  nginx -t 语法检查"),
        Line::from("  [s]  启动        [S] 停止        [x] 重启        [l] reload"),
        Line::from("  [e]  开机自启    [d] 取消自启"),
        Line::from("  [R]  刷新状态"),
        Line::from(""),
        Line::from(Span::styled(
            "  证书由你自己维护（推荐 acme.sh），sb-manager 只生成 server block 模板。",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "  生成配置后，编辑 ssl_certificate / ssl_certificate_key 路径，再 [t] 检查 → [l] reload。",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    f.render_widget(
        Paragraph::new(help)
            .block(Block::default().borders(Borders::ALL).title(" 操作 "))
            .wrap(Wrap { trim: true }),
        c[1],
    );
}
