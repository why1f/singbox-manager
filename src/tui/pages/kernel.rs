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
        .constraints([Constraint::Length(9), Constraint::Min(0)])
        .split(area);

    let status_lines = match &s.kernel {
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
            vec![
                Line::from(""),
                Line::from(vec![Span::raw("  状态: "), installed, Span::raw("    "), running, Span::raw("    "), enabled]),
                Line::from(""),
                Line::from(format!("  版本: {}", k.version.as_deref().unwrap_or("—"))),
                Line::from(format!("  路径: {}", k.binary_path.as_deref().unwrap_or("—"))),
                Line::from(""),
                Line::from(Span::styled(
                    match s.kernel_busy {
                        Some(op) => format!("  正在执行: {}  (请稍候，勿连续操作)", op),
                        None => "  空闲".into(),
                    },
                    Style::default().fg(if s.kernel_busy.is_some() { Color::Cyan } else { Color::DarkGray }),
                )),
            ]
        }
    };
    f.render_widget(
        Paragraph::new(status_lines).block(Block::default().borders(Borders::ALL).title(" sing-box 内核 ")),
        c[0],
    );

    let help = vec![
        Line::from(""),
        Line::from(Span::styled("  操作快捷键", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from("  [i]  安装官方版 (sing-box.app 脚本，不含 v2ray_api)"),
        Line::from("  [v]  安装 v2ray_api 版 (从本仓库 release 下载)"),
        Line::from("  [u]  卸载 (停服务 + 删二进制/unit；保留 /etc/sing-box)"),
        Line::from("  [s]  启动        [S] 停止        [x] 重启"),
        Line::from("  [e]  开机自启    [d] 取消自启"),
        Line::from("  [R]  刷新状态"),
        Line::from(""),
        Line::from(Span::styled(
            "  说明：流量统计依赖 experimental.v2ray_api (gRPC)，官方版不带此 build tag。",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "  v2ray_api 版由本仓库 workflow 基于上游源码 +with_v2ray_api 每日构建。",
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
