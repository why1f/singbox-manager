use ratatui::{Frame,layout::{Constraint,Rect},style::{Color,Modifier,Style},text::{Line,Span},
    widgets::{Block,Borders,Cell,Paragraph,Row,Table,Wrap}};
use crate::tui::app::AppState;
pub fn render(f: &mut Frame, area: Rect, s: &AppState) {
    let c = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([Constraint::Min(0),Constraint::Length(4)]).split(area);
    let hdr = Row::new(["Tag","协议","端口","用户数",""].map(|h|
        Cell::from(h).style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))));
    let rows: Vec<Row> = s.nodes.iter().enumerate().map(|(i, n)| {
        let sel = i == s.node_table.selected;
        let style = if sel { Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD) } else { Style::default() };
        Row::new(vec![
            Cell::from(n.tag.clone()),
            Cell::from(n.protocol.to_string()),
            Cell::from(n.listen_port.to_string()),
            Cell::from(n.user_count.to_string()),
            Cell::from(""),
        ]).style(style)
    }).collect();
    f.render_widget(Table::new(rows,[
        Constraint::Length(22),   // Tag
        Constraint::Length(16),   // 协议
        Constraint::Length(8),    // 端口
        Constraint::Length(10),   // 用户数
        Constraint::Min(0),       // 占位留白
    ]).header(hdr).block(Block::default().borders(Borders::ALL).title(" 节点列表 ")), c[0]);

    let sel = s.nodes.get(s.node_table.selected)
        .map(|n| format!("  选中: {}  协议: {}  端口: {}", n.tag, n.protocol, n.listen_port))
        .unwrap_or_else(|| "  (无节点)".into());
    f.render_widget(Paragraph::new(vec![
        Line::from(sel),
        Line::from(Span::styled(
            "  [a]添加  [E]编辑  [d]删除  [↑↓/jk]选择",
            Style::default().fg(Color::DarkGray))),
    ]).block(Block::default().borders(Borders::ALL).title(" 操作 ")).wrap(Wrap{trim:true}), c[1]);
}
