use crate::tui::app::AppState;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Tabs},
    Frame,
};
pub fn render(f: &mut Frame, area: Rect, s: &AppState) {
    f.render_widget(
        Tabs::new(vec![
            Line::from(" 仪表盘[1] "),
            Line::from(" 用户[2] "),
            Line::from(" 节点[3] "),
            Line::from(" 日志[4] "),
            Line::from(" 内核[5] "),
            Line::from(" 订阅[6] "),
        ])
        .select(s.page.index())
        .block(Block::default().borders(Borders::BOTTOM))
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().fg(Color::DarkGray)),
        area,
    );
}
