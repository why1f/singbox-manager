use crate::tui::app::AppState;
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};
pub fn render(f: &mut Frame, area: Rect, s: &AppState) {
    let h = area.height.saturating_sub(2) as usize;
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        "[f] journalctl -u sing-box -f   (Ctrl-C 退回)",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));
    if let Some(sub) = &s.last_subscription {
        lines.push(Line::from(Span::styled(
            "[SUB] 最近一次订阅导出",
            Style::default().fg(Color::Cyan),
        )));
        for line in sub.lines() {
            lines.push(Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(Color::White),
            )));
        }
        lines.push(Line::from(""));
    }
    let remain = h.saturating_sub(lines.len());
    let skip = s.log_lines.len().saturating_sub(remain);
    lines.extend(s.log_lines.iter().skip(skip).map(|l| {
        let c = if l.contains("ERROR") || l.contains("错误") {
            Color::Red
        } else if l.contains("WARN") || l.contains("警告") {
            Color::Yellow
        } else {
            Color::White
        };
        Line::from(Span::styled(l.clone(), Style::default().fg(c)))
    }));
    f.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" 日志 / 输出 "),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}
