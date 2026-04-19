use ratatui::{Frame,layout::Rect,style::{Color,Style},text::{Line,Span},widgets::Paragraph};
use crate::tui::app::{AppState,StatusLevel};
pub fn render(f: &mut Frame, area: Rect, s: &AppState) {
    let span = if let Some((ref t,ref lv)) = s.status_msg {
        let c=match lv{StatusLevel::Warn=>Color::Yellow,StatusLevel::Error=>Color::Red};
        Span::styled(format!(" {} ",t),Style::default().fg(c))
    } else {
        Span::styled(format!(" sb v{}  用户:{}  节点:{}  [Tab]切换 [q]退出 [R]刷新",
            env!("CARGO_PKG_VERSION"),s.users.len(),s.nodes.len()),Style::default().fg(Color::DarkGray))
    };
    f.render_widget(Paragraph::new(Line::from(span)), area);
}
