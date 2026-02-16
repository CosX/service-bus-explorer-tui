use ratatui::prelude::*;
use ratatui::widgets::*;
use ratatui::Frame;

use crate::app::{ActiveModal, App};

use super::sanitize::sanitize_for_terminal;

fn mask_secret_ascii_keep_suffix(input: &str, suffix_chars: usize) -> String {
    if input.is_empty() {
        return String::new();
    }

    // This app treats input_cursor as a byte offset; connection strings are ASCII.
    // Keep output strictly ASCII to avoid cursor-position drift.
    let len = input.len();
    let suffix = suffix_chars.min(len);
    let (prefix, tail) = input.split_at(len - suffix);
    format!("{}{}", "*".repeat(prefix.len()), tail)
}

fn redact_connection_string_for_preview(conn_str: &str) -> String {
    // Extract Endpoint and SharedAccessKeyName for a safe summary.
    let mut endpoint: Option<&str> = None;
    let mut key_name: Option<&str> = None;

    for part in conn_str.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((k, v)) = part.split_once('=') {
            match k.trim() {
                "Endpoint" => endpoint = Some(v.trim()),
                "SharedAccessKeyName" => key_name = Some(v.trim()),
                _ => {}
            }
        }
    }

    match (endpoint, key_name) {
        (Some(ep), Some(kn)) => format!(
            "Endpoint={}; SharedAccessKeyName={}; SharedAccessKey=***",
            ep, kn
        ),
        (Some(ep), None) => format!("Endpoint={}; SharedAccessKey=***", ep),
        _ => "(redacted SAS connection)".to_string(),
    }
}

pub fn render_modal(frame: &mut Frame, app: &mut App) {
    match &app.modal.clone() {
        ActiveModal::ConnectionModeSelect => render_connection_mode_select(frame),
        ActiveModal::ConnectionInput => render_connection_input(frame, app),
        ActiveModal::ConnectionList => render_connection_list(frame, app),
        ActiveModal::AzureAdNamespaceInput => render_azure_ad_input(frame, app),
        ActiveModal::SendMessage => render_form(frame, app, "Send Message", "F2 to send"),
        ActiveModal::EditResend => render_form(frame, app, "Edit & Resend", "F2 to resend"),
        ActiveModal::CreateQueue => render_form(frame, app, "Create Queue", "F2 to create"),
        ActiveModal::CreateTopic => render_form(frame, app, "Create Topic", "F2 to create"),
        ActiveModal::CreateSubscription => {
            render_form(frame, app, "Create Subscription", "F2 to create")
        }
        ActiveModal::ConfirmDelete(path) => render_confirm_delete(frame, path),
        ActiveModal::ConfirmBulkResend {
            entity_path, count, ..
        } => {
            render_confirm_bulk(
                frame,
                "Resend Peeked DLQ Messages",
                &format!(
                    "Resend {} peeked dead-letter messages back to '{}'?\nOriginals will be removed from DLQ.",
                    count, entity_path
                ),
                Color::Yellow,
            );
        }
        ActiveModal::ConfirmBulkDelete {
            entity_path,
            count,
            is_dlq,
            ..
        } => {
            let target = if *is_dlq { "DLQ" } else { "main queue" };
            render_confirm_bulk(
                frame,
                "Bulk Delete Messages",
                &format!(
                    "Destructively delete up to {} messages from {} of '{}'?\nThis cannot be undone.",
                    count, target, entity_path
                ),
                Color::Red,
            );
        }
        ActiveModal::PeekCountInput => render_peek_count_input(frame, app),
        ActiveModal::ClearOptions { entity_path, .. } => {
            render_clear_options(frame, entity_path);
        }
        ActiveModal::Help | ActiveModal::None => {}
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

/// Like centered_rect but uses absolute width (percentage) and absolute height (rows).
fn centered_rect_abs_height(percent_x: u16, height: u16, area: Rect) -> Rect {
    let h = height.min(area.height);
    let top = area.height.saturating_sub(h) / 2;
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(top),
            Constraint::Length(h),
            Constraint::Min(0),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn render_connection_input(frame: &mut Frame, app: &App) {
    let area = centered_rect(70, 20, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Connect — Enter Connection String ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(3)])
        .margin(1)
        .split(inner);

    let hint = Paragraph::new(
        "Paste your Service Bus connection string (masked) (Enter to connect, Esc to cancel)",
    )
    .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(hint, layout[0]);

    let masked = mask_secret_ascii_keep_suffix(app.input_buffer.as_str(), 4);
    let input = Paragraph::new(masked)
        .style(Style::default().fg(Color::White))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow)),
        );
    frame.render_widget(input, layout[1]);

    // Show cursor
    let cursor_x = layout[1].x + app.input_cursor as u16 + 1;
    let cursor_y = layout[1].y + 1;
    frame.set_cursor_position((cursor_x, cursor_y));
}

fn render_connection_list(frame: &mut Frame, app: &App) {
    let area = centered_rect(60, 50, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Saved Connections (n=new, d=delete, Enter=connect) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let items: Vec<ListItem> = app
        .config
        .connections
        .iter()
        .enumerate()
        .map(|(idx, conn)| {
            let style = if idx == app.input_field_index {
                Style::default().bg(Color::DarkGray).fg(Color::White).bold()
            } else {
                Style::default()
            };
            let detail = if conn.is_azure_ad() {
                format!("[AD] {}", conn.namespace.as_deref().unwrap_or("?"))
            } else {
                let preview = redact_connection_string_for_preview(
                    conn.connection_string.as_deref().unwrap_or(""),
                );
                format!("[SAS] {}…", truncate(&preview, 55))
            };
            ListItem::new(Line::from(Span::styled(
                format!("  {} — {}", conn.name, detail),
                style,
            )))
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

fn render_connection_mode_select(frame: &mut Frame) {
    let area = centered_rect_abs_height(50, 9, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Connect — Choose Auth Method ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let text = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  [1] ", Style::default().fg(Color::Yellow).bold()),
            Span::raw("Connection String (SAS)"),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  [2] ", Style::default().fg(Color::Yellow).bold()),
            Span::raw("Azure AD / Entra ID"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Esc to cancel",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let paragraph = Paragraph::new(text);
    frame.render_widget(paragraph, inner);
}

fn render_azure_ad_input(frame: &mut Frame, app: &App) {
    let area = centered_rect(70, 20, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Connect — Azure AD (Entra ID) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Length(3)])
        .margin(1)
        .split(inner);

    let hint = Paragraph::new(
        "Enter namespace (e.g. mynamespace or mynamespace.servicebus.windows.net)\nUses az login / Azure CLI credentials",
    )
    .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(hint, layout[0]);

    let input = Paragraph::new(app.input_buffer.as_str())
        .style(Style::default().fg(Color::White))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Magenta)),
        );
    frame.render_widget(input, layout[1]);

    let cursor_x = layout[1].x + app.input_cursor as u16 + 1;
    let cursor_y = layout[1].y + 1;
    frame.set_cursor_position((cursor_x, cursor_y));
}

fn render_form(frame: &mut Frame, app: &mut App, title: &str, hint: &str) {
    let san_ml = |s: &str| sanitize_for_terminal(s, true);

    // Check if the first field is a Body field (SendMessage / EditResend forms).
    let has_body = app
        .input_fields
        .first()
        .map(|(l, _)| l == "Body")
        .unwrap_or(false);

    if has_body {
        render_form_with_body(frame, app, title, hint, &san_ml);
    } else {
        render_form_flat(frame, app, title, hint);
    }
}

/// Form layout for Send/EditResend: multiline body area + single-line property fields.
fn render_form_with_body(
    frame: &mut Frame,
    app: &mut App,
    title: &str,
    hint: &str,
    san_ml: &dyn Fn(&str) -> String,
) {
    // Properties = fields 1..N, each needs 2 rows (label + value).
    let prop_count = app.input_fields.len().saturating_sub(1);
    let props_height = (prop_count as u16) * 2;
    // body area (bordered, min 8) + properties + hint + outer block borders (2) + margin (2)
    let min_height = 10 + props_height + 1 + 2 + 2;
    // Use 80% of terminal height, but at least min_height
    let desired = (frame.area().height * 80 / 100).max(min_height);
    let area = centered_rect_abs_height(70, desired, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(format!(" {} ", title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let form_layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Min(8),                    // body area (bordered)
            Constraint::Length(props_height),       // property fields
            Constraint::Length(1),                  // hint line
        ])
        .split(inner);

    let body_area = form_layout[0];
    let props_area = form_layout[1];
    let hint_area = form_layout[2];

    // ── Body field (index 0) ──
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
        } else if body_val.is_empty() {
            String::new()
        } else {
            san_ml(&pretty_print_body(body_val))
        };
        let body_widget = Paragraph::new(display_body)
            .style(Style::default().fg(Color::White))
            .wrap(Wrap { trim: false });

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

    // ── Property fields (1..N) ──
    let prop_constraints: Vec<Constraint> = (1..app.input_fields.len())
        .flat_map(|_| vec![Constraint::Length(1), Constraint::Length(1)])
        .collect();
    let prop_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(prop_constraints)
        .split(props_area);

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

    // ── Hint line ──
    let hint_widget = Paragraph::new(format!(
        "Tab fields · ↑↓←→ navigate · Enter newline (body) · {} · Esc cancel",
        hint
    ))
    .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(hint_widget, hint_area);
}

/// Flat form layout for Create* modals (no body field).
fn render_form_flat(frame: &mut Frame, app: &App, title: &str, hint: &str) {
    let field_count = app.input_fields.len();
    // Each field needs 2 rows (label + value), plus hint line, block borders (2), layout margin (2)
    let rows_needed = (field_count as u16) * 2 + 1 + 2 + 2;
    let area = centered_rect_abs_height(70, rows_needed, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(format!(" {} ", title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut constraints: Vec<Constraint> = app
        .input_fields
        .iter()
        .flat_map(|_| vec![Constraint::Length(1), Constraint::Length(1)])
        .collect();
    constraints.push(Constraint::Length(1)); // hint line
    constraints.push(Constraint::Min(0));

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(constraints)
        .split(inner);

    for (idx, (label, value)) in app.input_fields.iter().enumerate() {
        let label_idx = idx * 2;
        let value_idx = idx * 2 + 1;

        if label_idx >= layout.len() || value_idx >= layout.len() {
            break;
        }

        let is_active = idx == app.input_field_index;

        let label_style = if is_active {
            Style::default().fg(Color::Cyan).bold()
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let label_widget = Paragraph::new(format!("{}:", label)).style(label_style);
        frame.render_widget(label_widget, layout[label_idx]);

        let value_style = if is_active {
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

        let value_widget = Paragraph::new(display_val).style(value_style);
        frame.render_widget(value_widget, layout[value_idx]);
    }

    // Hint line
    let hint_idx = app.input_fields.len() * 2;
    if hint_idx < layout.len() {
        let hint_widget = Paragraph::new(format!(
            "Tab/↑↓ navigate · ←→/Home/End cursor · {} · Esc cancel",
            hint
        ))
        .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(hint_widget, layout[hint_idx]);
    }
}

fn pretty_print_body(body: &str) -> String {
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(body) {
        serde_json::to_string_pretty(&val).unwrap_or_else(|_| body.to_string())
    } else {
        body.to_string()
    }
}

fn render_confirm_delete(frame: &mut Frame, path: &str) {
    let area = centered_rect(50, 20, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Confirm Delete ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let text = Paragraph::new(vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("Delete '{}'?", path),
            Style::default().fg(Color::Red).bold(),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Press 'y' to confirm, 'n' or Esc to cancel",
            Style::default().fg(Color::DarkGray),
        )),
    ])
    .alignment(Alignment::Center);
    frame.render_widget(text, inner);
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}…", &s[..max_len])
    }
}

fn render_confirm_bulk(frame: &mut Frame, title: &str, message: &str, color: Color) {
    let area = centered_rect(55, 25, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(format!(" {} ", title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines = vec![Line::from("")];
    for line in message.lines() {
        lines.push(Line::from(Span::styled(
            line.to_string(),
            Style::default().fg(color).bold(),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Press 'y' to confirm, 'n' or Esc to cancel",
        Style::default().fg(Color::DarkGray),
    )));

    let text = Paragraph::new(lines).alignment(Alignment::Center);
    frame.render_widget(text, inner);
}

fn render_peek_count_input(frame: &mut Frame, app: &App) {
    let area = centered_rect(45, 20, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Peek Messages ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .margin(1)
        .split(inner);

    let label =
        Paragraph::new("How many messages to peek?").style(Style::default().fg(Color::White));
    frame.render_widget(label, layout[0]);

    let input = Paragraph::new(app.input_buffer.as_str())
        .style(Style::default().fg(Color::White))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow)),
        );
    frame.render_widget(input, layout[2]);

    let hint =
        Paragraph::new("Enter to peek · Esc to cancel").style(Style::default().fg(Color::DarkGray));
    frame.render_widget(hint, layout[3]);

    // Cursor
    let cursor_x = layout[2].x + app.input_cursor as u16 + 1;
    let cursor_y = layout[2].y + 1;
    frame.set_cursor_position((cursor_x, cursor_y));
}

fn render_clear_options(frame: &mut Frame, entity_path: &str) {
    let area = centered_rect(58, 35, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Clear Entity ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let entity_display = if entity_path.len() > 40 {
        format!("...{}", &entity_path[entity_path.len() - 37..])
    } else {
        entity_path.to_string()
    };

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            entity_display,
            Style::default().fg(Color::White).bold(),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  [D] ", Style::default().fg(Color::Red).bold()),
            Span::styled(
                "Delete ALL active messages",
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  [L] ", Style::default().fg(Color::Red).bold()),
            Span::styled(
                "Delete ALL dead-letter messages",
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  [R] ", Style::default().fg(Color::Yellow).bold()),
            Span::styled(
                "Resend ALL DLQ → main entity",
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Esc to cancel",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let text = Paragraph::new(lines).alignment(Alignment::Center);
    frame.render_widget(text, inner);
}
