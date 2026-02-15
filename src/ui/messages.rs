use ratatui::prelude::*;
use ratatui::widgets::*;
use ratatui::Frame;

use crate::app::{App, FocusPanel, MessageTab};

use super::sanitize::sanitize_for_terminal;

pub fn render_messages(frame: &mut Frame, app: &mut App, area: Rect) {
    let is_focused = app.focus == FocusPanel::Messages;
    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    // Tab titles
    let msg_tab_style = if app.message_tab == MessageTab::Messages {
        Style::default().fg(Color::Cyan).bold()
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let dlq_tab_style = if app.message_tab == MessageTab::DeadLetter {
        Style::default().fg(Color::Red).bold()
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let title = Line::from(vec![
        Span::raw(" "),
        Span::styled("[1] Messages", msg_tab_style),
        Span::raw(" | "),
        Span::styled("[2] Dead-letter", dlq_tab_style),
        Span::raw(" "),
    ]);

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    // If we have a selected message detail, show it
    if app.selected_message_detail.is_some() {
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if app.detail_editing {
            render_detail_edit(frame, app, inner);
        } else {
            render_detail_readonly(frame, app, inner);
        }
        return;
    }

    let messages = match app.message_tab {
        MessageTab::Messages => &app.messages,
        MessageTab::DeadLetter => &app.dlq_messages,
    };

    if messages.is_empty() {
        let msg = Paragraph::new("No messages. Press 'p' on an entity to peek active messages or press 'd' to peek dead-letter messages.")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        frame.render_widget(msg, area);
        return;
    }

    let inner = block.inner(area);

    // Build table rows
    let header = Row::new(vec!["#", "Message ID", "Seq #", "Size", "Enqueued"])
        .style(Style::default().fg(Color::Yellow).bold())
        .bottom_margin(1);

    let rows: Vec<Row> = messages
        .iter()
        .enumerate()
        .map(|(idx, msg)| {
            let style = if idx == app.message_selected && is_focused {
                Style::default().bg(Color::DarkGray).fg(Color::White)
            } else {
                Style::default()
            };

            Row::new(vec![
                (idx + 1).to_string(),
                sanitize_for_terminal(
                    &msg.broker_properties
                        .message_id
                        .clone()
                        .unwrap_or_else(|| "-".to_string()),
                    false,
                ),
                msg.broker_properties
                    .sequence_number
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                msg.broker_properties
                    .size
                    .map(format_size)
                    .unwrap_or_else(|| "-".to_string()),
                sanitize_for_terminal(
                    &msg.broker_properties
                        .enqueued_time_utc
                        .clone()
                        .unwrap_or_else(|| "-".to_string()),
                    false,
                ),
            ])
            .style(style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(4),
            Constraint::Percentage(30),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Percentage(30),
        ],
    )
    .header(header)
    .block(Block::default())
    .column_spacing(1);

    // Persist scroll offset across frames for natural scrolling
    app.message_table_state.select(Some(app.message_selected));

    // Layout: table + hint bar
    let msg_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(inner);

    let hint_text = if app.message_tab == MessageTab::DeadLetter {
        "R=Resend All  D=Delete All  Enter=View  e=Edit & Resend"
    } else {
        "D=Delete All  Enter=View  e=Edit & Resend"
    };
    let hint = Paragraph::new(hint_text).style(Style::default().fg(Color::DarkGray));

    frame.render_widget(block, area);
    frame.render_stateful_widget(table, msg_layout[0], &mut app.message_table_state);
    frame.render_widget(hint, msg_layout[1]);
}

fn render_detail_readonly(frame: &mut Frame, app: &mut App, inner: Rect) {
    let msg = app.selected_message_detail.as_ref().unwrap();

    let san = |s: &str| sanitize_for_terminal(s, false);
    let san_ml = |s: &str| sanitize_for_terminal(s, true);

    // Properties table
    let mut props_rows = Vec::new();
    if let Some(ref id) = msg.broker_properties.message_id {
        props_rows.push(Row::new(vec!["Message ID".to_string(), san(id)]));
    }
    if let Some(ref id) = msg.broker_properties.correlation_id {
        props_rows.push(Row::new(vec!["Correlation ID".to_string(), san(id)]));
    }
    if let Some(seq) = msg.broker_properties.sequence_number {
        let seq_str = seq.to_string();
        props_rows.push(Row::new(vec!["Sequence #".to_string(), san(&seq_str)]));
    }
    if let Some(ref t) = msg.broker_properties.enqueued_time_utc {
        props_rows.push(Row::new(vec!["Enqueued".to_string(), san(t)]));
    }
    if let Some(count) = msg.broker_properties.delivery_count {
        let count_str = count.to_string();
        props_rows.push(Row::new(vec![
            "Delivery Count".to_string(),
            san(&count_str),
        ]));
    }
    if let Some(ref label) = msg.broker_properties.label {
        props_rows.push(Row::new(vec!["Label".to_string(), san(label)]));
    }
    if let Some(ref src) = msg.broker_properties.dead_letter_source {
        props_rows.push(Row::new(vec!["DLQ Source".to_string(), san(src)]));
    }
    if let Some(ref reason) = msg.broker_properties.dead_letter_reason {
        props_rows.push(Row::new(vec!["DLQ Reason".to_string(), san(reason)]));
    }
    if let Some(ref desc) = msg.broker_properties.dead_letter_error_description {
        props_rows.push(Row::new(vec!["DLQ Error".to_string(), san(desc)]));
    }
    for (k, v) in &msg.custom_properties {
        props_rows.push(Row::new(vec![san(k), san(v)]));
    }

    let props_height = (props_rows.len() as u16 + 2).max(4); // rows + border

    let detail_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(props_height), Constraint::Min(3)])
        .split(inner);

    let props_table = Table::new(
        props_rows,
        [Constraint::Percentage(30), Constraint::Percentage(70)],
    )
    .block(
        Block::default()
            .title(" Properties (e = edit & resend · Esc = close) ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow)),
    );
    frame.render_widget(props_table, detail_layout[0]);

    let body = san_ml(&pretty_print_body(&msg.body));
    let body_lines = body.lines().count() as u16;
    let body_inner = Block::default()
        .title(" Body (j/k to scroll · Esc = close) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let body_viewport = body_inner.inner(detail_layout[1]).height;
    // Clamp scroll so we don't scroll past the end
    if body_lines > body_viewport {
        app.detail_body_scroll = app
            .detail_body_scroll
            .min(body_lines.saturating_sub(body_viewport));
    } else {
        app.detail_body_scroll = 0;
    }
    let body_widget = Paragraph::new(body)
        .block(body_inner)
        .wrap(Wrap { trim: false })
        .scroll((app.detail_body_scroll, 0));
    frame.render_widget(body_widget, detail_layout[1]);
}

/// WYSIWYG inline edit view — fields 1..N at top, body (field 0) at bottom.
fn render_detail_edit(frame: &mut Frame, app: &mut App, inner: Rect) {
    let san_ml = |s: &str| sanitize_for_terminal(s, true);

    // input_fields layout:
    //   0: Body
    //   1: Content-Type
    //   2: Message ID
    //   3: Correlation ID
    //   4: Session ID
    //   5: Label
    //   6: TTL
    //   7: Custom Properties
    let prop_field_count = app.input_fields.len().saturating_sub(1); // fields 1..N
    let props_height = (prop_field_count as u16 * 2 + 2).max(4); // rows for prop fields + border

    // Check if source message has DLQ info to show read-only
    let dlq_info: Vec<(&str, String)> = if let Some(ref msg) = app.selected_message_detail {
        let mut info = Vec::new();
        if let Some(ref src) = msg.broker_properties.dead_letter_source {
            info.push(("DLQ Source", src.clone()));
        }
        if let Some(ref reason) = msg.broker_properties.dead_letter_reason {
            info.push(("DLQ Reason", reason.clone()));
        }
        if let Some(ref desc) = msg.broker_properties.dead_letter_error_description {
            info.push(("DLQ Error", desc.clone()));
        }
        info
    } else {
        Vec::new()
    };
    let dlq_height = if dlq_info.is_empty() {
        0u16
    } else {
        dlq_info.len() as u16 + 2 // rows + border
    };

    let detail_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(dlq_height),   // DLQ info (read-only)
            Constraint::Length(props_height), // editable properties
            Constraint::Min(5),               // editable body
            Constraint::Length(1),            // hint bar
        ])
        .split(inner);

    // ── DLQ info (read-only) ──
    if !dlq_info.is_empty() {
        let dlq_block = Block::default()
            .title(" Dead-letter Info ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red));
        let dlq_inner = dlq_block.inner(detail_layout[0]);
        frame.render_widget(dlq_block, detail_layout[0]);

        let dlq_rows: Vec<Row> = dlq_info
            .iter()
            .map(|(k, v)| Row::new(vec![k.to_string(), v.clone()]))
            .collect();
        let dlq_table = Table::new(
            dlq_rows,
            [Constraint::Percentage(25), Constraint::Percentage(75)],
        );
        frame.render_widget(dlq_table, dlq_inner);
    }

    let props_area = detail_layout[1];
    let body_area = detail_layout[2];
    let hint_area = detail_layout[3];

    // ── Editable properties (fields 1..N) ──
    let props_block = Block::default()
        .title(" Properties (editable) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let props_inner = props_block.inner(props_area);
    frame.render_widget(props_block, props_area);

    let prop_constraints: Vec<Constraint> = (1..app.input_fields.len())
        .flat_map(|_| vec![Constraint::Length(1), Constraint::Length(1)])
        .collect();
    let prop_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(prop_constraints)
        .split(props_inner);

    for field_idx in 1..app.input_fields.len() {
        let (ref label, ref value) = app.input_fields[field_idx];
        let row = (field_idx - 1) * 2;
        let label_row = row;
        let value_row = row + 1;

        if label_row >= prop_layout.len() || value_row >= prop_layout.len() {
            break;
        }

        let is_active = field_idx == app.input_field_index;

        let label_style = if is_active {
            Style::default().fg(Color::Cyan).bold()
        } else {
            Style::default().fg(Color::DarkGray)
        };
        frame.render_widget(
            Paragraph::new(format!("{}:", label)).style(label_style),
            prop_layout[label_row],
        );

        let val_style = if is_active {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::Gray)
        };
        let display_val = if is_active {
            let cursor = app.form_cursor.min(value.len());
            let (before, after) = value.split_at(cursor);
            format!("{}▏{}", before, after)
        } else {
            value.clone()
        };
        frame.render_widget(
            Paragraph::new(display_val).style(val_style),
            prop_layout[value_row],
        );
    }

    // ── Editable body (field 0) ──
    let body_is_active = app.input_field_index == 0;
    let body_border_style = if body_is_active {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::Yellow)
    };
    let body_block = Block::default()
        .title(if body_is_active {
            " Body (editing) "
        } else {
            " Body "
        })
        .borders(Borders::ALL)
        .border_style(body_border_style);
    let body_inner = body_block.inner(body_area);
    frame.render_widget(body_block, body_area);

    if let Some((_, ref body_val)) = app.input_fields.first() {
        let display_body = if body_is_active {
            let cursor = app.form_cursor.min(body_val.len());
            let (before, after) = body_val.split_at(cursor);
            san_ml(&format!("{}▏{}", before, after))
        } else {
            san_ml(&pretty_print_body(body_val))
        };
        let body_widget = Paragraph::new(display_body)
            .style(Style::default().fg(Color::White))
            .wrap(Wrap { trim: false });

        // Auto-scroll to keep cursor visible
        if body_is_active {
            let cursor_pos = app.form_cursor.min(body_val.len());
            let cursor_line = body_val[..cursor_pos].matches('\n').count() as u16;
            let visible = body_inner.height.saturating_sub(1);
            if cursor_line < app.body_scroll {
                app.body_scroll = cursor_line;
            } else if cursor_line >= app.body_scroll + visible.max(1) {
                app.body_scroll = cursor_line.saturating_sub(visible.saturating_sub(1));
            }
            frame.render_widget(body_widget.scroll((app.body_scroll, 0)), body_inner);
        } else {
            app.body_scroll = 0;
            frame.render_widget(body_widget, body_inner);
        }
    }

    // ── Hint bar ──
    let hint = Paragraph::new(
        "Tab fields · ↑↓←→ navigate · Enter newline (body) · F2 resend · Esc cancel",
    )
    .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(hint, hint_area);
}

fn pretty_print_body(body: &str) -> String {
    // Try to parse as JSON and pretty-print
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(body) {
        serde_json::to_string_pretty(&val).unwrap_or_else(|_| body.to_string())
    } else {
        body.to_string()
    }
}

fn format_size(bytes: i64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}
