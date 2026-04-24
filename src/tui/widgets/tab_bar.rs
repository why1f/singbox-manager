use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::tui::app::AppState;

pub const TAB_LABELS: [&str; 6] = [
    " 仪表盘[1] ",
    " 用户[2] ",
    " 节点[3] ",
    " 日志[4] ",
    " 内核[5] ",
    " 订阅[6] ",
];

const TAB_DIVIDER: &str = " │ ";

pub fn render(f: &mut Frame, area: Rect, s: &AppState) {
    let mut spans = Vec::new();
    for (idx, label) in TAB_LABELS.iter().enumerate() {
        let style = if idx == s.page.index() {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        spans.push(Span::styled(*label, style));
        if idx + 1 != TAB_LABELS.len() {
            spans.push(Span::styled(TAB_DIVIDER, Style::default().fg(Color::Gray)));
        }
    }

    f.render_widget(Block::default().borders(Borders::BOTTOM), area);
    let line_area = Rect { height: 1, ..area };
    f.render_widget(Paragraph::new(Line::from(spans)), line_area);
}

pub fn hit_test(area: Rect, column: u16, row: u16) -> Option<usize> {
    if row != area.y {
        return None;
    }
    let mut x = area.x;
    for (idx, label) in TAB_LABELS.iter().enumerate() {
        let w = label.chars().count() as u16;
        if column >= x && column < x.saturating_add(w) {
            return Some(idx);
        }
        x = x.saturating_add(w);
        if idx + 1 != TAB_LABELS.len() {
            x = x.saturating_add(TAB_DIVIDER.chars().count() as u16);
        }
    }
    None
}
