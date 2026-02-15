use ratatui::prelude::*;
use ratatui::widgets::*;
use ratatui::Frame;

use crate::app::{App, FocusPanel};
use crate::client::models::EntityType;

pub fn render_tree(frame: &mut Frame, app: &mut App, area: Rect) {
    let is_focused = app.focus == FocusPanel::Tree;
    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .title(" Entities ")
        .borders(Borders::ALL)
        .border_style(border_style);

    if app.flat_nodes.is_empty() {
        let placeholder = Paragraph::new("No connection. Press 'c' to connect.")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        frame.render_widget(placeholder, area);
        return;
    }

    let inner = block.inner(area);

    // Build list items from flat nodes
    let items: Vec<ListItem> = app
        .flat_nodes
        .iter()
        .enumerate()
        .map(|(idx, node)| {
            let indent = "  ".repeat(node.depth);
            let icon = match node.entity_type {
                EntityType::Namespace => "🏢",
                EntityType::QueueFolder => "📁",
                EntityType::TopicFolder => "📁",
                EntityType::Queue => "📬",
                EntityType::Topic => "📢",
                EntityType::SubscriptionFolder => "📁",
                EntityType::Subscription => "📥",
                EntityType::DeadLetterQueue => "💀",
            };

            let expand_indicator = if node.has_children {
                if node.expanded {
                    "▼ "
                } else {
                    "▶ "
                }
            } else {
                "  "
            };

            let count_str = match (node.message_count, node.dlq_count) {
                (Some(msg), Some(dlq)) if dlq > 0 => {
                    format!(" [{}] (💀{})", msg, dlq)
                }
                (Some(msg), _) => format!(" [{}]", msg),
                _ => String::new(),
            };

            let line = format!(
                "{}{}{} {}{}",
                indent, expand_indicator, icon, node.label, count_str
            );

            let style = if idx == app.tree_selected && is_focused {
                Style::default().bg(Color::DarkGray).fg(Color::White).bold()
            } else if idx == app.tree_selected {
                Style::default().fg(Color::Yellow)
            } else {
                match node.entity_type {
                    EntityType::DeadLetterQueue => Style::default().fg(Color::Red),
                    EntityType::QueueFolder
                    | EntityType::TopicFolder
                    | EntityType::SubscriptionFolder => Style::default().fg(Color::Blue),
                    _ => Style::default(),
                }
            };

            ListItem::new(Line::from(Span::styled(line, style)))
        })
        .collect();

    // Scrolling: ensure selected item is visible
    let visible_height = inner.height as usize;
    let _offset = if app.tree_selected >= visible_height {
        app.tree_selected - visible_height + 1
    } else {
        0
    };

    let list = List::new(items)
        .block(Block::default())
        .highlight_style(Style::default());

    // Persist scroll offset across frames for natural scrolling
    app.tree_list_state.select(Some(app.tree_selected));

    frame.render_widget(block, area);
    frame.render_stateful_widget(list, inner, &mut app.tree_list_state);
}
