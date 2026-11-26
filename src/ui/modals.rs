use ratatui::prelude::*;
use ratatui::widgets::*;
use ratatui::Frame;

use crossterm::event::KeyCode;

use crate::app::{ActiveModal, App, ClearAction, ClearScope};
use crate::mouse::{HitMap, Target};

use super::buttons::{render_buttons, Button};

/// The last line of `area`, where dialogs put their buttons.
fn bottom_row(area: Rect) -> Rect {
    Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(1),
        width: area.width,
        height: area.height.min(1),
    }
}

/// Yes / No buttons on the bottom line of a confirmation dialog.
fn render_yes_no(frame: &mut Frame, inner: Rect, hits: &mut HitMap, yes_color: Color) {
    render_buttons(
        frame,
        bottom_row(inner),
        hits,
        &[
            Button::new("Yes  y", KeyCode::Char('y'), yes_color),
            Button::new("No  n", KeyCode::Char('n'), Color::Gray),
        ],
        Alignment::Center,
    );
}

use super::sanitize::{floor_char_boundary, sanitize_for_terminal, value_with_cursor};

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
        ActiveModal::ConnectionModeSelect => render_connection_mode_select(frame, &mut app.hits),
        ActiveModal::ConnectionInput => render_connection_input(frame, app),
        ActiveModal::ConnectionList => render_connection_list(frame, app),
        ActiveModal::ConnectionSwitch => render_connection_switch(frame, app),
        ActiveModal::AzureAdNamespaceInput => render_azure_ad_input(frame, app),
        ActiveModal::SendMessage => render_form(frame, app, "Send Message", "Send"),
        ActiveModal::EditResend => render_form(frame, app, "Edit & Resend", "Resend"),
        ActiveModal::CreateQueue => render_form(frame, app, "Create Queue", "Create"),
        ActiveModal::CreateTopic => render_form(frame, app, "Create Topic", "Create"),
        ActiveModal::CreateSubscription => render_form(frame, app, "Create Subscription", "Create"),
        ActiveModal::EditSubscriptionFilter => {
            render_form(frame, app, "Edit Subscription Filter", "Update")
        }
        ActiveModal::ConfirmDelete(path) => render_confirm_delete(frame, path, &mut app.hits),
        ActiveModal::ConfirmBulkResend {
            entity_path,
            count,
            is_topic,
        } => {
            let target = if *is_topic {
                entity_path.as_str()
            } else {
                crate::client::entity_path::send_target(entity_path)
            };
            let warning = app.resend_fan_out_warning(entity_path, *is_topic);
            render_confirm_bulk(
                frame,
                "Resend Peeked DLQ Messages",
                &format!(
                    "Resend {} peeked dead-letter messages to '{}'?\nEach original is removed from the DLQ once its copy is sent.",
                    count, target
                ),
                warning.as_deref(),
                Color::Yellow,
                &mut app.hits,
            );
        }
        ActiveModal::ConfirmBulkDelete {
            entity_path,
            count,
            is_dlq,
            ..
        } => {
            let target = if *is_dlq { "DLQ" } else { "active messages" };
            let warning = (!*is_dlq).then_some(
                "Finding them locks the messages ahead of them, which uses up one delivery attempt each.",
            );
            render_confirm_bulk(
                frame,
                "Delete Peeked Messages",
                &format!(
                    "Delete the {} peeked messages from the {} of '{}'?\nOther messages are left in place. This cannot be undone.",
                    count, target, entity_path
                ),
                warning,
                Color::Red,
                &mut app.hits,
            );
        }
        ActiveModal::ConfirmSingleDelete {
            entity_path,
            sequence_number,
            is_dlq,
        } => {
            let target = if *is_dlq { "DLQ" } else { "queue" };
            render_confirm_bulk(
                frame,
                "Delete Message",
                &format!(
                    "Delete message seq #{} from {} of '{}'?\nThis cannot be undone.",
                    sequence_number, target, entity_path
                ),
                None,
                Color::Red,
                &mut app.hits,
            );
        }
        ActiveModal::PeekCountInput => render_peek_count_input(frame, app),
        ActiveModal::ClearOptions { scope } => {
            let warning = match scope {
                ClearScope::Entity(path) => app.resend_fan_out_warning(path, false),
                ClearScope::Topic(topic) => app.resend_fan_out_warning(topic, true),
                ClearScope::AllQueues | ClearScope::AllTopics => None,
            };
            render_clear_options(frame, scope, warning.as_deref(), &mut app.hits);
        }
        ActiveModal::ConfirmClearBatch { scope, action } => {
            render_confirm_clear_batch(frame, scope, *action, &mut app.hits);
        }
        ActiveModal::ConfirmPurgeAllDlq => {
            render_confirm_purge_all_dlq(frame, &mut app.hits);
        }
        ActiveModal::NamespaceDiscovery { state } => render_namespace_discovery(frame, app, state),
        ActiveModal::CopySelectConnection => render_copy_select_connection(frame, app),
        ActiveModal::CopySelectEntity => render_copy_select_entity(frame, app),
        ActiveModal::CopyEditMessage => {
            let dest = app
                .copy_destination_entity
                .as_deref()
                .unwrap_or("destination");
            let conn = app
                .copy_dest_connection_name
                .as_deref()
                .unwrap_or("connection");
            let title = format!("Copy to {} @ {}", dest, conn);
            render_form(frame, app, &title, "Copy")
        }
        ActiveModal::Help | ActiveModal::MetricsDetail | ActiveModal::None => {}
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

fn render_popup_block(frame: &mut Frame, area: Rect, title: String, border_color: Color) -> Rect {
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

fn set_single_line_cursor(frame: &mut Frame, input_area: Rect, cursor: usize) {
    let cursor_x = input_area.x + cursor as u16 + 1;
    let cursor_y = input_area.y + 1;
    frame.set_cursor_position((cursor_x, cursor_y));
}

fn render_shortcut_hints(frame: &mut Frame, area: Rect, shortcuts: &[(&str, &str)]) {
    let mut spans = Vec::with_capacity(shortcuts.len() * 2);
    for (key, text) in shortcuts {
        spans.push(Span::styled(
            *key,
            Style::default().fg(Color::Yellow).bold(),
        ));
        spans.push(Span::styled(*text, Style::default().fg(Color::DarkGray)));
    }

    let hints = Paragraph::new(vec![Line::from(spans)]);
    frame.render_widget(hints, area);
}

fn render_centered_lines(frame: &mut Frame, area: Rect, lines: Vec<Line<'_>>) {
    let text = Paragraph::new(lines)
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true });
    frame.render_widget(text, area);
}

fn render_connection_input(frame: &mut Frame, app: &App) {
    let area = centered_rect(70, 20, frame.area());
    let inner = render_popup_block(
        frame,
        area,
        " Connect — Enter Connection String ".to_string(),
        Color::Cyan,
    );

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

    set_single_line_cursor(frame, layout[1], app.input_cursor);
}

fn render_connection_list(frame: &mut Frame, app: &mut App) {
    let area = centered_rect(60, 50, frame.area());
    let inner = render_popup_block(
        frame,
        area,
        " Saved Connections (n=new, d=delete, Enter=connect) ".to_string(),
        Color::Cyan,
    );

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

    let count = items.len();
    let list = List::new(items);
    frame.render_widget(list, inner);
    for row in 0..count.min(inner.height as usize) {
        app.hits
            .add_row(inner, row as u16, Target::DialogListRow(row));
    }
}

fn render_connection_mode_select(frame: &mut Frame, hits: &mut HitMap) {
    let area = centered_rect_abs_height(50, 9, frame.area());
    let inner = render_popup_block(
        frame,
        area,
        " Connect — Choose Auth Method ".to_string(),
        Color::Cyan,
    );

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
    hits.add_row(inner, 1, Target::Key(KeyCode::Char('1')));
    hits.add_row(inner, 3, Target::Key(KeyCode::Char('2')));
    hits.add_row(inner, 5, Target::Key(KeyCode::Esc));
}

fn render_azure_ad_input(frame: &mut Frame, app: &App) {
    let area = centered_rect(70, 20, frame.area());
    let inner = render_popup_block(
        frame,
        area,
        " Connect — Azure AD (Entra ID) ".to_string(),
        Color::Magenta,
    );

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

    set_single_line_cursor(frame, layout[1], app.input_cursor);
}

fn render_connection_switch(frame: &mut Frame, app: &mut App) {
    let area = centered_rect(50, 40, frame.area());
    let inner = render_popup_block(
        frame,
        area,
        " Connection Management ".to_string(),
        Color::Cyan,
    );

    let current_conn = app.connection_name.as_deref().unwrap_or("Unknown");

    let text = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("Current connection: ", Style::default().fg(Color::DarkGray)),
            Span::styled(current_conn, Style::default().fg(Color::White).bold()),
        ]),
        Line::from(""),
        Line::from(""),
        Line::from(vec![
            Span::styled("  [D] ", Style::default().fg(Color::Yellow).bold()),
            Span::raw("Disconnect"),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  [S] ", Style::default().fg(Color::Yellow).bold()),
            Span::raw("Switch connection"),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  [C/Esc] ", Style::default().fg(Color::Yellow).bold()),
            Span::raw("Cancel (stay connected)"),
        ]),
    ];

    render_centered_lines(frame, inner, text);
    render_buttons(
        frame,
        bottom_row(inner),
        &mut app.hits,
        &[
            Button::new("Disconnect", KeyCode::Char('d'), Color::Yellow),
            Button::new("Switch", KeyCode::Char('s'), Color::Cyan),
            Button::new("Cancel", KeyCode::Esc, Color::Gray),
        ],
        Alignment::Center,
    );
}

/// `action` labels the submit button (F2).
fn render_form(frame: &mut Frame, app: &mut App, title: &str, action: &str) {
    let san_ml = |s: &str| sanitize_for_terminal(s, true);

    // Check if the first field is a Body field (SendMessage / EditResend forms).
    let has_body = app
        .input_fields
        .first()
        .map(|(l, _)| l == "Body")
        .unwrap_or(false);

    if has_body {
        render_form_with_body(frame, app, title, action, &san_ml);
    } else {
        render_form_flat(frame, app, title, action);
    }
}

/// Form layout for Send/EditResend: multiline body area + single-line property fields.
fn render_form_with_body(
    frame: &mut Frame,
    app: &mut App,
    title: &str,
    action: &str,
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
            Constraint::Min(8),               // body area (bordered)
            Constraint::Length(props_height), // property fields
            Constraint::Length(1),            // hint line
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
    app.hits.add(body_area, Target::FormField(0));

    if let Some((_, ref body_val)) = app.input_fields.first() {
        let display_body = if body_is_active {
            let cursor = floor_char_boundary(body_val, app.form_cursor);
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
            let cursor_pos = floor_char_boundary(body_val, app.form_cursor);
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
            value_with_cursor(value, app.form_cursor)
        } else {
            sanitize_for_terminal(value, false)
        };
        frame.render_widget(
            Paragraph::new(display_val).style(val_style),
            prop_layout[value_row],
        );
        app.hits
            .add(prop_layout[label_row], Target::FormField(field_idx));
        app.hits
            .add(prop_layout[value_row], Target::FormField(field_idx));
    }

    // ── Hint line: key hints left, buttons right ──
    let hint_widget = Paragraph::new("Tab/click fields · ↑↓←→ navigate · Enter newline (body)")
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(hint_widget, hint_area);
    render_form_buttons(frame, hint_area, &mut app.hits, action);
}

/// Submit (F2) and Cancel (Esc) buttons, right-aligned.
fn render_form_buttons(frame: &mut Frame, area: Rect, hits: &mut HitMap, action: &str) {
    let submit = format!("{} F2", action);
    render_buttons(
        frame,
        area,
        hits,
        &[
            Button::new(&submit, KeyCode::F(2), Color::Green),
            Button::new("Cancel Esc", KeyCode::Esc, Color::Gray),
        ],
        Alignment::Right,
    );
}

/// Flat form layout for Create* modals (no body field).
fn render_form_flat(frame: &mut Frame, app: &mut App, title: &str, action: &str) {
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
            value_with_cursor(value, app.form_cursor)
        } else {
            sanitize_for_terminal(value, false)
        };

        let value_widget = Paragraph::new(display_val).style(value_style);
        frame.render_widget(value_widget, layout[value_idx]);
        app.hits.add(layout[label_idx], Target::FormField(idx));
        app.hits.add(layout[value_idx], Target::FormField(idx));
    }

    // Hint line: key hints left, buttons right
    let hint_idx = app.input_fields.len() * 2;
    if hint_idx < layout.len() {
        let hint_widget = Paragraph::new("Tab/↑↓/click fields · ←→/Home/End cursor")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(hint_widget, layout[hint_idx]);
        render_form_buttons(frame, layout[hint_idx], &mut app.hits, action);
    }
}

fn pretty_print_body(body: &str) -> String {
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(body) {
        serde_json::to_string_pretty(&val).unwrap_or_else(|_| body.to_string())
    } else {
        body.to_string()
    }
}

fn render_confirm_delete(frame: &mut Frame, path: &str, hits: &mut HitMap) {
    let area = centered_rect(50, 20, frame.area());
    let inner = render_popup_block(frame, area, " Confirm Delete ".to_string(), Color::Red);

    render_centered_lines(
        frame,
        inner,
        vec![
            Line::from(""),
            Line::from(Span::styled(
                format!("Delete '{}'{}", path, "?"),
                Style::default().fg(Color::Red).bold(),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Press 'y' to confirm, 'n' or Esc to cancel",
                Style::default().fg(Color::DarkGray),
            )),
        ],
    );
    render_yes_no(frame, inner, hits, Color::Red);
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}…", &s[..max_len])
    }
}

fn render_confirm_bulk(
    frame: &mut Frame,
    title: &str,
    message: &str,
    warning: Option<&str>,
    color: Color,
    hits: &mut HitMap,
) {
    let height = if warning.is_some() { 40 } else { 25 };
    let area = centered_rect(60, height, frame.area());
    let inner = render_popup_block(frame, area, format!(" {} ", title), color);

    let mut lines = vec![Line::from("")];
    for line in message.lines() {
        lines.push(Line::from(Span::styled(
            sanitize_for_terminal(line, false),
            Style::default().fg(color).bold(),
        )));
    }
    if let Some(warning) = warning {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("⚠ {}", sanitize_for_terminal(warning, false)),
            Style::default().fg(Color::Red).bold(),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Press 'y' to confirm, 'n' or Esc to cancel",
        Style::default().fg(Color::DarkGray),
    )));

    render_centered_lines(frame, inner, lines);
    render_yes_no(frame, inner, hits, color);
}

fn render_peek_count_input(frame: &mut Frame, app: &mut App) {
    let area = centered_rect(55, 30, frame.area());
    let inner = render_popup_block(frame, area, " Peek Messages ".to_string(), Color::Cyan);

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

    render_buttons(
        frame,
        layout[3],
        &mut app.hits,
        &[
            Button::new("Peek  Enter", KeyCode::Enter, Color::Cyan),
            Button::new("Cancel  Esc", KeyCode::Esc, Color::Gray),
        ],
        Alignment::Left,
    );

    // Peeking active messages costs each one a delivery attempt.
    if !app.peek_dlq {
        let warning = Paragraph::new(
            "⚠ Peek locks each message and uses up one delivery attempt. A message already at \
             MaxDeliveryCount is dead-lettered by the peek, which then stops.",
        )
        .style(Style::default().fg(Color::Yellow))
        .wrap(Wrap { trim: true });
        frame.render_widget(warning, layout[4]);
    }

    set_single_line_cursor(frame, layout[2], app.input_cursor);
}

fn render_clear_options(
    frame: &mut Frame,
    scope: &ClearScope,
    resend_warning: Option<&str>,
    hits: &mut HitMap,
) {
    let height = if resend_warning.is_some() { 48 } else { 35 };
    let area = centered_rect(58, height, frame.area());
    let title = if scope.is_batch() {
        " Clear Entities "
    } else {
        " Clear Entity "
    };
    let inner = render_popup_block(frame, area, title.to_string(), Color::Yellow);

    let target = scope.label();
    let target_display = if target.len() > 40 {
        format!("...{}", &target[target.len() - 37..])
    } else {
        target.to_string()
    };

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            target_display,
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
    ];
    let mut lines = lines;
    if let Some(warning) = resend_warning {
        lines.push(Line::from(Span::styled(
            format!("⚠ [R] {}", sanitize_for_terminal(warning, false)),
            Style::default().fg(Color::Red),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Esc to cancel",
        Style::default().fg(Color::DarkGray),
    )));

    render_centered_lines(frame, inner, lines);
    render_buttons(
        frame,
        bottom_row(inner),
        hits,
        &[
            Button::new("Del active", KeyCode::Char('d'), Color::Red),
            Button::new("Del DLQ", KeyCode::Char('l'), Color::Red),
            Button::new("Resend DLQ", KeyCode::Char('r'), Color::Yellow),
            Button::new("Cancel", KeyCode::Esc, Color::Gray),
        ],
        Alignment::Center,
    );
}

/// Second confirmation for a namespace-wide clear (Queues / Topics folder).
fn render_confirm_clear_batch(
    frame: &mut Frame,
    scope: &ClearScope,
    action: ClearAction,
    hits: &mut HitMap,
) {
    let area = centered_rect(62, 32, frame.area());
    let (title, accent) = match action {
        ClearAction::DeleteActive | ClearAction::DeleteDlq => (" Clear ALL Entities ", Color::Red),
        ClearAction::ResendDlq => (" Resend ALL Dead-Letter Queues ", Color::Yellow),
    };
    let inner = render_popup_block(frame, area, title.to_string(), accent);

    let target = match scope {
        ClearScope::AllQueues => "every queue in the namespace",
        ClearScope::AllTopics => "every subscription of every topic",
        _ => scope.label(),
    };

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            action.description(),
            Style::default().fg(accent).bold(),
        )),
        Line::from(Span::styled(
            format!("across {}.", target),
            Style::default().fg(accent).bold(),
        )),
        Line::from(""),
        Line::from(Span::styled(
            if action == ClearAction::ResendDlq && *scope == ClearScope::AllTopics {
                "Each message is resent to its topic and reaches EVERY matching subscription, \
                 not only the one it was dead-lettered from. This cannot be undone."
            } else {
                "This action cannot be undone."
            },
            Style::default().fg(Color::Yellow),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  [Y] ", Style::default().fg(accent).bold()),
            Span::styled("Yes, run it", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("  [N] ", Style::default().fg(Color::Green).bold()),
            Span::styled("Cancel", Style::default().fg(Color::White)),
        ]),
        Line::from(""),
    ];

    render_centered_lines(frame, inner, lines);
    render_yes_no(frame, inner, hits, accent);
}

fn render_confirm_purge_all_dlq(frame: &mut Frame, hits: &mut HitMap) {
    let area = centered_rect(58, 30, frame.area());
    let inner = render_popup_block(
        frame,
        area,
        " Purge ALL Dead-Letter Queues ".to_string(),
        Color::Red,
    );

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "This will DELETE all dead-letter messages",
            Style::default().fg(Color::Red).bold(),
        )),
        Line::from(Span::styled(
            "across every queue and subscription",
            Style::default().fg(Color::Red).bold(),
        )),
        Line::from(Span::styled(
            "in the entire namespace.",
            Style::default().fg(Color::Red).bold(),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "This action cannot be undone.",
            Style::default().fg(Color::Yellow),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  [Y] ", Style::default().fg(Color::Red).bold()),
            Span::styled("Yes, purge all DLQs", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("  [N] ", Style::default().fg(Color::Green).bold()),
            Span::styled("Cancel", Style::default().fg(Color::White)),
        ]),
        Line::from(""),
    ];

    render_centered_lines(frame, inner, lines);
    render_yes_no(frame, inner, hits, Color::Red);
}

fn render_namespace_discovery(
    frame: &mut Frame,
    app: &mut App,
    state: &crate::app::DiscoveryState,
) {
    use crate::app::DiscoveryState;
    match state {
        DiscoveryState::Loading => render_discovery_loading(frame),
        DiscoveryState::List => render_namespace_list(frame, app),
        DiscoveryState::Error(msg) => render_discovery_error(frame, msg, &mut app.hits),
    }
}

fn render_discovery_loading(frame: &mut Frame) {
    let area = centered_rect(50, 20, frame.area());
    let inner = render_popup_block(
        frame,
        area,
        " Azure AD — Discovering Namespaces ".to_string(),
        Color::Magenta,
    );

    let lines = vec![
        Line::from(""),
        Line::from(""),
        Line::from(Span::styled(
            "🔍 Discovering available Service Bus namespaces...",
            Style::default().fg(Color::Cyan).bold(),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Querying Azure subscriptions via Azure CLI credentials",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Esc to cancel",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    render_centered_lines(frame, inner, lines);
}

/// "Enter manually" / "Cancel" buttons used by the discovery dialogs.
fn render_manual_or_cancel(frame: &mut Frame, inner: Rect, hits: &mut HitMap) {
    render_buttons(
        frame,
        bottom_row(inner),
        hits,
        &[
            Button::new("Enter manually  m", KeyCode::Char('m'), Color::Yellow),
            Button::new("Cancel  Esc", KeyCode::Esc, Color::Gray),
        ],
        Alignment::Center,
    );
}

fn render_discovery_error(frame: &mut Frame, msg: &str, hits: &mut HitMap) {
    let area = centered_rect(60, 30, frame.area());
    let inner = render_popup_block(
        frame,
        area,
        " Namespace Discovery Failed ".to_string(),
        Color::Red,
    );

    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "⚠ Failed to discover namespaces",
            Style::default().fg(Color::Red).bold(),
        )),
        Line::from(""),
    ];

    // Add error message lines
    for line in msg.lines() {
        lines.push(Line::from(Span::styled(
            line.to_string(),
            Style::default().fg(Color::White),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("Press ", Style::default().fg(Color::DarkGray)),
        Span::styled("'m'", Style::default().fg(Color::Yellow).bold()),
        Span::styled(
            " to enter namespace manually",
            Style::default().fg(Color::DarkGray),
        ),
    ]));
    lines.push(Line::from(Span::styled(
        "or Esc to cancel",
        Style::default().fg(Color::DarkGray),
    )));

    render_centered_lines(frame, inner, lines);
    render_manual_or_cancel(frame, inner, hits);
}

fn render_namespace_list(frame: &mut Frame, app: &mut App) {
    let area = centered_rect(80, 70, frame.area());
    let inner = render_popup_block(
        frame,
        area,
        " Azure AD — Select Service Bus Namespace ".to_string(),
        Color::Magenta,
    );

    if app.discovered_namespaces.is_empty() {
        // No namespaces found
        let lines = vec![
            Line::from(""),
            Line::from(""),
            Line::from(Span::styled(
                "No Service Bus namespaces found in your Azure subscriptions",
                Style::default().fg(Color::Yellow).bold(),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Make sure you are logged in with 'az login' and have access to subscriptions",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(""),
            Line::from(""),
            Line::from(vec![
                Span::styled("Press ", Style::default().fg(Color::DarkGray)),
                Span::styled("'m'", Style::default().fg(Color::Yellow).bold()),
                Span::styled(
                    " to enter namespace manually",
                    Style::default().fg(Color::DarkGray),
                ),
            ]),
            Line::from(Span::styled(
                "or Esc to cancel",
                Style::default().fg(Color::DarkGray),
            )),
        ];
        render_centered_lines(frame, inner, lines);
        render_manual_or_cancel(frame, inner, &mut app.hits);
        return;
    }

    // Split into header and content area
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // header with count + warnings
            Constraint::Min(3),    // namespace list
            Constraint::Length(2), // hints
        ])
        .margin(1)
        .split(inner);

    // Header
    let mut header_lines = vec![Line::from(Span::styled(
        format!("Found {} namespace(s)", app.discovered_namespaces.len()),
        Style::default().fg(Color::Cyan),
    ))];

    if !app.discovery_warnings.is_empty() {
        header_lines.push(Line::from(Span::styled(
            format!(
                "⚠ {} subscription(s) had errors",
                app.discovery_warnings.len()
            ),
            Style::default().fg(Color::Yellow),
        )));
    }

    let header = Paragraph::new(header_lines);
    frame.render_widget(header, layout[0]);

    // Namespace list
    let mut items: Vec<ListItem> = Vec::new();

    // Group by subscription
    let mut by_subscription: std::collections::HashMap<
        String,
        Vec<&crate::client::resource_manager::DiscoveredNamespace>,
    > = std::collections::HashMap::new();

    for ns in &app.discovered_namespaces {
        by_subscription
            .entry(ns.subscription_name.clone())
            .or_default()
            .push(ns);
    }

    let mut sorted_subs: Vec<_> = by_subscription.keys().collect();
    sorted_subs.sort();

    // Which namespace (if any) each visual row shows; subscription headers
    // are rows too, so they are not clickable.
    let mut row_namespace: Vec<Option<usize>> = Vec::new();
    let mut idx = 0;
    for sub_name in sorted_subs {
        let namespaces = &by_subscription[sub_name];

        // Subscription header
        row_namespace.push(None);
        items.push(ListItem::new(Line::from(Span::styled(
            format!("  {}", sub_name),
            Style::default().fg(Color::Blue).bold(),
        ))));

        for ns in namespaces {
            let is_selected = idx == app.namespace_list_state;

            let status_icon = match ns.status.as_str() {
                "Active" => "✓",
                "Disabled" | "Disabling" => "✗",
                _ => "?",
            };

            let status_color = match ns.status.as_str() {
                "Active" => Color::Green,
                "Disabled" | "Disabling" => Color::Red,
                _ => Color::Yellow,
            };

            let line_style = if is_selected {
                Style::default().bg(Color::DarkGray).fg(Color::White)
            } else {
                Style::default()
            };

            let line = Line::from(vec![
                Span::styled("    ", line_style),
                Span::styled(
                    status_icon,
                    Style::default()
                        .fg(status_color)
                        .add_modifier(line_style.add_modifier),
                ),
                Span::styled(" ", line_style),
                Span::styled(&ns.name, line_style.fg(Color::White).bold()),
                Span::styled("  ", line_style),
                Span::styled(format!("[{}]", ns.location), line_style.fg(Color::DarkGray)),
                Span::styled("  ", line_style),
                Span::styled(&ns.status, line_style.fg(status_color)),
            ]);

            items.push(ListItem::new(line));
            row_namespace.push(Some(idx));
            idx += 1;
        }
    }

    let list = List::new(items);
    frame.render_widget(list, layout[1]);
    for (row, ns) in row_namespace.iter().enumerate() {
        if row >= layout[1].height as usize {
            break;
        }
        if let Some(ns) = ns {
            app.hits
                .add_row(layout[1], row as u16, Target::DialogListRow(*ns));
        }
    }

    render_shortcut_hints(
        frame,
        layout[2],
        &[
            ("↑↓/j/k", " navigate  "),
            ("Enter", " connect  "),
            ("m", " manual  "),
            ("Esc", " cancel"),
        ],
    );
}

/// Register the visible rows of a scrolled single-line list.
fn register_list_rows(hits: &mut HitMap, area: Rect, offset: usize, count: usize) {
    for row in 0..area.height {
        let index = offset + row as usize;
        if index >= count {
            break;
        }
        hits.add_row(area, row, Target::DialogListRow(index));
    }
}

fn render_copy_select_connection(frame: &mut Frame, app: &mut App) {
    let area = centered_rect(70, 60, frame.area());
    let inner = render_popup_block(
        frame,
        area,
        " Copy Message — Select Destination Connection ".to_string(),
        Color::Cyan,
    );

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // header
            Constraint::Min(3),    // connection list
            Constraint::Length(1), // footer hints
        ])
        .margin(1)
        .split(inner);

    // Header
    let header = Paragraph::new("Select a destination connection to copy this message to.")
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(header, layout[0]);

    // Connection list
    let items: Vec<ListItem> = app
        .config
        .connections
        .iter()
        .map(|conn| {
            let auth_type = if conn.is_azure_ad() {
                "Azure AD"
            } else {
                "SAS"
            };
            let display_name = if let Some(namespace) = &conn.namespace {
                format!("{} — [{}]", conn.name, namespace)
            } else {
                conn.name.clone()
            };
            ListItem::new(Line::from(Span::raw(format!(
                "  {} ({})",
                display_name, auth_type
            ))))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White).bold());

    let count = app.config.connections.len();
    app.copy_connection_list_state
        .select(Some(app.input_field_index));
    frame.render_stateful_widget(list, layout[1], &mut app.copy_connection_list_state);
    let offset = app.copy_connection_list_state.offset();
    register_list_rows(&mut app.hits, layout[1], offset, count);

    render_shortcut_hints(
        frame,
        layout[2],
        &[
            ("↑↓/j/k", " navigate | "),
            ("Enter", " select | "),
            ("Esc", " cancel"),
        ],
    );
}

fn render_copy_select_entity(frame: &mut Frame, app: &mut App) {
    let area = centered_rect(70, 60, frame.area());
    let connection_name = app
        .copy_dest_connection_name
        .as_deref()
        .unwrap_or("Unknown");
    let inner = render_popup_block(
        frame,
        area,
        format!(
            " Copy Message — Select Destination Entity [{}] ",
            connection_name
        ),
        Color::Cyan,
    );

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // header (2 lines)
            Constraint::Min(3),    // entity list
            Constraint::Length(1), // footer hints
        ])
        .margin(1)
        .split(inner);

    // Header
    let source_entity = app
        .selected_entity()
        .map(|(path, _)| path.to_string())
        .unwrap_or_else(|| "(unknown)".to_string());
    let header = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("Source: ", Style::default().fg(Color::DarkGray)),
            Span::styled(source_entity, Style::default().fg(Color::Yellow)),
        ]),
        Line::from(Span::styled(
            "Select destination queue or topic, or press 's' to use same entity name.",
            Style::default().fg(Color::DarkGray),
        )),
    ]);
    frame.render_widget(header, layout[0]);

    // Entity list
    // Use copy_dest_entities from app state
    let has_entities = !app.copy_dest_entities.is_empty();

    if !has_entities {
        let loading = Paragraph::new("Loading entities...")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(loading, layout[1]);
    } else {
        // Render entity list with type icons
        // Use copy_dest_entities from app state
        use crate::client::models::EntityType;

        let items: Vec<ListItem> = app
            .copy_dest_entities
            .iter()
            .map(|(path, entity_type)| {
                let icon = match entity_type {
                    EntityType::Queue => "📬",
                    EntityType::Topic => "📢",
                    _ => "",
                };
                ListItem::new(Line::from(Span::raw(format!("  {} {}", icon, path))))
            })
            .collect();

        if items.is_empty() {
            let empty_msg = Paragraph::new("No queues or topics found")
                .style(Style::default().fg(Color::DarkGray))
                .alignment(Alignment::Center);
            frame.render_widget(empty_msg, layout[1]);
        } else {
            let list = List::new(items)
                .highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White).bold());

            let count = app.copy_dest_entities.len();
            app.copy_entity_list_state
                .select(Some(app.copy_entity_selected));
            frame.render_stateful_widget(list, layout[1], &mut app.copy_entity_list_state);
            let offset = app.copy_entity_list_state.offset();
            register_list_rows(&mut app.hits, layout[1], offset, count);
        }
    }

    render_shortcut_hints(
        frame,
        layout[2],
        &[
            ("↑↓/j/k", " navigate | "),
            ("Enter", " select | "),
            ("s", " use source name | "),
            ("Esc", " cancel"),
        ],
    );
}
