use crate::tui::app::AppState;
use crate::tui::forms::protocol_supports_port_reuse;
use ratatui::{
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap},
    Frame,
};
pub fn render(f: &mut Frame, area: Rect, s: &AppState) {
    let c = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(4)])
        .split(area);
    let hdr = Row::new(["Tag", "协议", "端口", "用户数", "端口复用", ""].map(|h| {
        Cell::from(h).style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    }));
    let rows: Vec<Row> = s
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let sel = i == s.node_table.selected;
            let style = if sel {
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let proto = n.protocol.to_string();
            let reuse_cell = if protocol_supports_port_reuse(&proto) {
                let on = crate::core::config::get_node_meta(&n.tag)
                    .map(|m| m.port_reuse)
                    .unwrap_or(false);
                if on {
                    Cell::from("● 开").style(Style::default().fg(Color::Green))
                } else {
                    // 不染色：让它继承行样式，选中时是白字+灰底，可读
                    Cell::from("○ 关")
                }
            } else {
                Cell::from("─ 不支持")
            };
            Row::new(vec![
                Cell::from(n.tag.clone()),
                Cell::from(proto),
                Cell::from(n.listen_port.to_string()),
                Cell::from(n.user_count.to_string()),
                reuse_cell,
                Cell::from(""),
            ])
            .style(style)
        })
        .collect();
    f.render_widget(
        Table::new(
            rows,
            [
                Constraint::Length(22), // Tag
                Constraint::Length(16), // 协议
                Constraint::Length(8),  // 端口
                Constraint::Length(8),  // 用户数
                Constraint::Length(12), // 端口复用
                Constraint::Min(0),     // 占位留白
            ],
        )
        .header(hdr)
        .block(Block::default().borders(Borders::ALL).title(" 节点列表 ")),
        c[0],
    );

    let sel = s
        .nodes
        .get(s.node_table.selected)
        .map(|n| {
            let reuse = crate::core::config::get_node_meta(&n.tag)
                .map(|m| m.port_reuse)
                .unwrap_or(false);
            let port_part = if reuse {
                format!("内部 {} · 对外 443 (端口复用)", n.listen_port)
            } else {
                n.listen_port.to_string()
            };
            format!(
                "  选中: {}  协议: {}  端口: {}",
                n.tag, n.protocol, port_part
            )
        })
        .unwrap_or_else(|| "  (无节点)".into());
    f.render_widget(
        Paragraph::new(vec![
            Line::from(sel),
            Line::from(Span::styled(
                "  [a]添加  [E]编辑  [d]删除  [C]编辑 config.json  [↑↓/jk]选择",
                Style::default().fg(Color::DarkGray),
            )),
        ])
        .block(Block::default().borders(Borders::ALL).title(" 操作 "))
        .wrap(Wrap { trim: true }),
        c[1],
    );
}
