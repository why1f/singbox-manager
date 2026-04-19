use ratatui::{Frame,layout::{Constraint,Rect},style::{Color,Modifier,Style},
    widgets::{Block,Borders,Cell,Row,Table}};
use crate::tui::app::AppState;
pub fn render(f: &mut Frame, area: Rect, s: &AppState) {
    let hdr = Row::new(["Tag","协议","端口","用户数"].map(|h|
        Cell::from(h).style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))));
    let rows: Vec<Row> = s.nodes.iter().map(|n| Row::new(vec![
        Cell::from(n.tag.clone()),Cell::from(n.protocol.to_string()),
        Cell::from(n.listen_port.to_string()),Cell::from(n.user_count.to_string()),
    ])).collect();
    f.render_widget(Table::new(rows,[
        Constraint::Min(20),Constraint::Length(16),Constraint::Length(8),Constraint::Length(8),
    ]).header(hdr).block(Block::default().borders(Borders::ALL).title(" 节点列表 ")), area);
}
