use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use std::time::Duration;

use crate::app::{ActiveModal, App, FocusPanel, MessageTab};
use crate::client::models::EntityType;

/// Poll for input events and process them against app state.
/// Returns true if the app should continue running.
pub fn handle_events(app: &mut App) -> anyhow::Result<bool> {
    if event::poll(Duration::from_millis(100))? {
        if let Event::Key(key) = event::read()? {
            // If a background operation is running, Esc cancels it
            if app.bg_running && key.code == KeyCode::Esc {
                app.cancel_bg();
                app.set_status("Cancelling...");
                return Ok(app.running);
            }

            // If a modal is open, route to modal handler
            if app.modal != ActiveModal::None {
                handle_modal_input(app, key);
                return Ok(app.running);
            }

            // If inline editing is active, skip global keys — route directly to panel handler
            if app.detail_editing {
                handle_message_input(app, key);
                return Ok(app.running);
            }

            // Global keys
            match key.code {
                KeyCode::Char('q') if key.modifiers.is_empty() => {
                    app.running = false;
                    return Ok(false);
                }
                KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
                    app.running = false;
                    return Ok(false);
                }
                KeyCode::Char('?') => {
                    app.modal = ActiveModal::Help;
                    return Ok(true);
                }
                KeyCode::Char('c') if key.modifiers.is_empty() => {
                    if app.management.is_none() {
                        // Open connection flow
                        app.input_buffer.clear();
                        app.input_cursor = 0;
                        if app.config.connections.is_empty() {
                            app.modal = ActiveModal::ConnectionModeSelect;
                        } else {
                            app.modal = ActiveModal::ConnectionList;
                        }
                    }
                    return Ok(true);
                }
                KeyCode::Tab => {
                    app.focus = match app.focus {
                        FocusPanel::Tree => FocusPanel::Detail,
                        FocusPanel::Detail => FocusPanel::Messages,
                        FocusPanel::Messages => FocusPanel::Tree,
                    };
                    return Ok(true);
                }
                KeyCode::BackTab => {
                    app.focus = match app.focus {
                        FocusPanel::Tree => FocusPanel::Messages,
                        FocusPanel::Detail => FocusPanel::Tree,
                        FocusPanel::Messages => FocusPanel::Detail,
                    };
                    return Ok(true);
                }
                _ => {}
            }

            // Panel-specific keys
            match app.focus {
                FocusPanel::Tree => handle_tree_input(app, key),
                FocusPanel::Detail => handle_detail_input(app, key),
                FocusPanel::Messages => handle_message_input(app, key),
            }
        }
    }
    Ok(app.running)
}

fn handle_tree_input(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            if app.tree_selected > 0 {
                app.tree_selected -= 1;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.tree_selected + 1 < app.flat_nodes.len() {
                app.tree_selected += 1;
            }
        }
        KeyCode::Home | KeyCode::Char('g') => {
            app.tree_selected = 0;
        }
        KeyCode::End | KeyCode::Char('G') => {
            if !app.flat_nodes.is_empty() {
                app.tree_selected = app.flat_nodes.len() - 1;
            }
        }
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
            // Toggle expand for folders, or select entity
            if !app.flat_nodes.is_empty() {
                let node = &app.flat_nodes[app.tree_selected];
                if node.has_children {
                    app.toggle_expand();
                }
            }
        }
        KeyCode::Left | KeyCode::Char('h') => {
            // Collapse current node
            if !app.flat_nodes.is_empty() {
                let node = &app.flat_nodes[app.tree_selected];
                if node.expanded && node.has_children {
                    app.toggle_expand();
                }
            }
        }
        // 'r' = refresh (handled async in main loop via flag)
        KeyCode::Char('r') | KeyCode::F(5) => {
            if app.bg_running {
                app.set_status("A background operation is in progress...");
            } else {
                app.set_status("Refreshing...");
                // Trigger async refresh — handled in main loop
            }
        }
        // 's' = send message to selected entity
        KeyCode::Char('s') => {
            if app.bg_running {
                app.set_status("A background operation is in progress...");
            } else if let Some((_, entity_type)) = app.selected_entity() {
                match entity_type {
                    EntityType::Queue | EntityType::Topic => {
                        app.init_send_form();
                    }
                    _ => {
                        app.set_status("Select a queue or topic to send messages");
                    }
                }
            }
        }
        // 'p' = peek messages — prompt for count
        KeyCode::Char('p') => {
            if app.bg_running {
                app.set_status("A background operation is in progress...");
            } else if let Some((_, entity_type)) = app.selected_entity() {
                match entity_type {
                    EntityType::Queue | EntityType::Subscription => {
                        app.input_buffer = app.config.settings.peek_count.to_string();
                        app.input_cursor = app.input_buffer.len();
                        app.modal = ActiveModal::PeekCountInput;
                        app.peek_dlq = false;
                    }
                    _ => {
                        app.set_status("Select a queue or subscription to peek messages");
                    }
                }
            }
        }
        // 'n' = new entity
        KeyCode::Char('n') => {
            if app.bg_running {
                app.set_status("A background operation is in progress...");
            } else if !app.flat_nodes.is_empty() {
                let node = &app.flat_nodes[app.tree_selected];
                match node.entity_type {
                    EntityType::QueueFolder | EntityType::Queue => {
                        app.init_create_queue_form();
                    }
                    EntityType::TopicFolder | EntityType::Topic => {
                        app.init_create_topic_form();
                    }
                    EntityType::SubscriptionFolder | EntityType::Subscription => {
                        // Find the parent topic name
                        let topic = find_parent_topic(app);
                        if let Some(topic_name) = topic {
                            app.init_create_subscription_form(&topic_name);
                        }
                    }
                    _ => {
                        app.set_status("Navigate to a queue/topic folder to create entities");
                    }
                }
            }
        }
        // 'x' = delete selected entity
        KeyCode::Char('x') => {
            if app.bg_running {
                app.set_status("A background operation is in progress...");
            } else if let Some((
                path,
                entity_type @ (EntityType::Queue | EntityType::Topic | EntityType::Subscription),
            )) = app.selected_entity()
            {
                let _ = entity_type;
                let path = path.to_string();
                app.modal = ActiveModal::ConfirmDelete(path);
                app.input_buffer.clear();
            }
        }
        // 'd' = peek dead-letter queue for selected entity
        KeyCode::Char('d') => {
            if app.bg_running {
                app.set_status("A background operation is in progress...");
            } else if let Some((_, entity_type)) = app.selected_entity() {
                match entity_type {
                    EntityType::Queue | EntityType::Subscription | EntityType::Topic => {
                        app.input_buffer = app.config.settings.peek_count.to_string();
                        app.input_cursor = app.input_buffer.len();
                        app.modal = ActiveModal::PeekCountInput;
                        app.peek_dlq = true;
                    }
                    _ => {
                        app.set_status("Select a queue, topic, or subscription to peek its DLQ");
                    }
                }
            }
        }
        // 'P' (shift+p) = clear entity (choose delete or resend)
        KeyCode::Char('P') => {
            if app.bg_running {
                app.set_status("A background operation is in progress...");
            } else if let Some((path, entity_type)) = app.selected_entity() {
                match entity_type {
                    EntityType::Queue | EntityType::Subscription | EntityType::Topic => {
                        let entity_path = path.to_string();
                        let is_topic = *entity_type == EntityType::Topic;
                        app.modal = ActiveModal::ClearOptions {
                            entity_path: entity_path.clone(),
                            base_entity_path: entity_path,
                            is_topic,
                        };
                    }
                    _ => {
                        app.set_status("Select a queue, topic, or subscription to clear");
                    }
                }
            }
        }
        _ => {}
    }
}

fn handle_detail_input(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('1') => {
            app.message_tab = MessageTab::Messages;
            app.focus = FocusPanel::Messages;
        }
        KeyCode::Char('2') => {
            app.message_tab = MessageTab::DeadLetter;
            app.focus = FocusPanel::Messages;
        }
        _ => {}
    }
}

fn handle_message_input(app: &mut App, key: KeyEvent) {
    // If inline editing is active, route to the field editor
    if app.detail_editing {
        handle_detail_edit_input(app, key);
        return;
    }

    let messages = match app.message_tab {
        MessageTab::Messages => &app.messages,
        MessageTab::DeadLetter => &app.dlq_messages,
    };
    let len = messages.len();

    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            // Scroll body when viewing message detail, else navigate list
            if app.selected_message_detail.is_some() {
                app.detail_body_scroll = app.detail_body_scroll.saturating_sub(1);
            } else if app.message_selected > 0 {
                app.message_selected -= 1;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.selected_message_detail.is_some() {
                app.detail_body_scroll = app.detail_body_scroll.saturating_add(1);
            } else if app.message_selected + 1 < len {
                app.message_selected += 1;
            }
        }
        KeyCode::Enter => {
            // Show message detail
            let msgs = match app.message_tab {
                MessageTab::Messages => &app.messages,
                MessageTab::DeadLetter => &app.dlq_messages,
            };
            if let Some(msg) = msgs.get(app.message_selected) {
                app.selected_message_detail = Some(msg.clone());
                app.detail_body_scroll = 0;
            }
        }
        KeyCode::Char('1') => {
            app.message_tab = MessageTab::Messages;
            app.message_selected = 0;
        }
        KeyCode::Char('2') => {
            app.message_tab = MessageTab::DeadLetter;
            app.message_selected = 0;
        }
        // R = Bulk resend from DLQ back to main entity
        KeyCode::Char('R') => {
            if app.bg_running {
                app.set_status("A background operation is in progress...");
            } else if app.message_tab == MessageTab::DeadLetter {
                if let Some((path, entity_type)) = app.selected_entity() {
                    let base_path = path.to_string();
                    match entity_type {
                        EntityType::Queue | EntityType::Subscription | EntityType::Topic => {
                            let is_topic = *entity_type == EntityType::Topic;
                            let count = app.dlq_messages.len() as u32;
                            if count > 0 {
                                app.modal = ActiveModal::ConfirmBulkResend {
                                    entity_path: base_path,
                                    count,
                                    is_topic,
                                };
                            } else {
                                app.set_status("No DLQ messages to resend");
                            }
                        }
                        _ => {
                            app.set_status(
                                "Select a queue, topic, or subscription to resend DLQ messages",
                            );
                        }
                    }
                }
            } else {
                app.set_status("Switch to DLQ tab (2) to resend dead-letter messages");
            }
        }
        // D = Bulk delete visible messages
        KeyCode::Char('D') => {
            if app.bg_running {
                app.set_status("A background operation is in progress...");
            } else if let Some((path, entity_type)) = app.selected_entity() {
                match entity_type {
                    EntityType::Queue | EntityType::Subscription | EntityType::Topic => {
                        let is_dlq = app.message_tab == MessageTab::DeadLetter;
                        let is_topic = *entity_type == EntityType::Topic;
                        let msgs = if is_dlq {
                            &app.dlq_messages
                        } else {
                            &app.messages
                        };
                        let count = msgs.len() as u32;
                        if count > 0 {
                            app.modal = ActiveModal::ConfirmBulkDelete {
                                entity_path: path.to_string(),
                                count,
                                is_dlq,
                                is_topic,
                            };
                        } else {
                            app.set_status("No messages to delete");
                        }
                    }
                    _ => {
                        app.set_status("Select a queue, topic, or subscription");
                    }
                }
            }
        }
        // e = Edit & resend selected message
        KeyCode::Char('e') => {
            if app.selected_message_detail.is_some() {
                // Enter inline WYSIWYG edit mode
                app.init_detail_edit();
            } else {
                // No detail open — use list selection and enter inline edit
                let msg = match app.message_tab {
                    MessageTab::Messages => app.messages.get(app.message_selected).cloned(),
                    MessageTab::DeadLetter => app.dlq_messages.get(app.message_selected).cloned(),
                };
                if let Some(msg) = msg {
                    app.selected_message_detail = Some(msg);
                    app.init_detail_edit();
                } else {
                    app.set_status("No message selected");
                }
            }
        }
        KeyCode::Esc => {
            app.selected_message_detail = None;
            app.detail_body_scroll = 0;
        }
        _ => {}
    }
}

fn handle_modal_input(app: &mut App, key: KeyEvent) {
    match &app.modal {
        ActiveModal::Help => {
            // Any key closes help
            app.modal = ActiveModal::None;
        }
        ActiveModal::ConnectionModeSelect => match key.code {
            KeyCode::Char('1') | KeyCode::Char('s') | KeyCode::Char('S') => {
                // SAS connection string
                app.input_buffer.clear();
                app.input_cursor = 0;
                app.modal = ActiveModal::ConnectionInput;
            }
            KeyCode::Char('2') | KeyCode::Char('a') | KeyCode::Char('A') => {
                // Azure AD
                app.input_buffer.clear();
                app.input_cursor = 0;
                app.modal = ActiveModal::AzureAdNamespaceInput;
            }
            KeyCode::Esc => {
                app.modal = ActiveModal::None;
            }
            _ => {}
        },
        ActiveModal::AzureAdNamespaceInput => match key.code {
            KeyCode::Enter => {
                let ns = app.input_buffer.trim().to_string();
                if !ns.is_empty() {
                    // Append .servicebus.windows.net if user only typed the short name
                    let fqns = if ns.contains('.') {
                        ns.clone()
                    } else {
                        format!("{}.servicebus.windows.net", ns)
                    };
                    match app.connect_azure_ad(&fqns) {
                        Ok(_) => {
                            app.config
                                .add_azure_ad_connection(fqns.clone(), fqns.clone());
                            let _ = app.config.save();
                            app.connection_name = Some(fqns);
                            app.modal = ActiveModal::None;
                            app.set_status("Connected via Azure AD! Loading entities...");
                        }
                        Err(e) => {
                            app.set_error(format!("Azure AD connection failed: {}", e));
                        }
                    }
                }
            }
            KeyCode::Esc => {
                app.modal = ActiveModal::None;
            }
            KeyCode::Char(c) => {
                app.input_buffer.insert(app.input_cursor, c);
                app.input_cursor += 1;
            }
            KeyCode::Backspace => {
                if app.input_cursor > 0 {
                    app.input_cursor -= 1;
                    app.input_buffer.remove(app.input_cursor);
                }
            }
            KeyCode::Left => {
                if app.input_cursor > 0 {
                    app.input_cursor -= 1;
                }
            }
            KeyCode::Right => {
                if app.input_cursor < app.input_buffer.len() {
                    app.input_cursor += 1;
                }
            }
            KeyCode::Home => app.input_cursor = 0,
            KeyCode::End => app.input_cursor = app.input_buffer.len(),
            _ => {}
        },
        ActiveModal::ConfirmDelete(_) => match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                app.set_status("Deleting...");
                // Actual deletion handled in main loop
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                app.modal = ActiveModal::None;
            }
            _ => {}
        },
        ActiveModal::ConfirmBulkResend { .. } => match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                app.set_status("Bulk resending...");
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                app.modal = ActiveModal::None;
            }
            _ => {}
        },
        ActiveModal::ConfirmBulkDelete { .. } => match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                app.set_status("Bulk deleting...");
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                app.modal = ActiveModal::None;
            }
            _ => {}
        },
        ActiveModal::PeekCountInput => match key.code {
            KeyCode::Enter => {
                if let Ok(count) = app.input_buffer.trim().parse::<i32>() {
                    if count > 0 {
                        app.pending_peek_count = Some(count);
                        app.modal = ActiveModal::None;
                        app.set_status("Peeking messages...");
                    } else {
                        app.set_error("Count must be a positive number");
                    }
                } else {
                    app.set_error("Invalid number");
                }
            }
            KeyCode::Esc => {
                app.modal = ActiveModal::None;
            }
            KeyCode::Char(c) if c.is_ascii_digit() => {
                app.input_buffer.insert(app.input_cursor, c);
                app.input_cursor += 1;
            }
            KeyCode::Backspace => {
                if app.input_cursor > 0 {
                    app.input_cursor -= 1;
                    app.input_buffer.remove(app.input_cursor);
                }
            }
            KeyCode::Left => {
                if app.input_cursor > 0 {
                    app.input_cursor -= 1;
                }
            }
            KeyCode::Right => {
                if app.input_cursor < app.input_buffer.len() {
                    app.input_cursor += 1;
                }
            }
            KeyCode::Home => app.input_cursor = 0,
            KeyCode::End => app.input_cursor = app.input_buffer.len(),
            _ => {}
        },
        ActiveModal::ClearOptions { .. } => match key.code {
            KeyCode::Char('d') | KeyCode::Char('D') => {
                app.set_status("Clearing (delete)...");
            }
            KeyCode::Char('l') | KeyCode::Char('L') => {
                app.set_status("Clearing (delete DLQ)...");
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                app.set_status("Clearing (resend)...");
            }
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                app.modal = ActiveModal::None;
            }
            _ => {}
        },
        ActiveModal::ConnectionList => match key.code {
            KeyCode::Esc => {
                app.modal = ActiveModal::None;
            }
            KeyCode::Char('n') => {
                app.input_buffer.clear();
                app.input_cursor = 0;
                app.modal = ActiveModal::ConnectionModeSelect;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if app.input_field_index > 0 {
                    app.input_field_index -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if app.input_field_index + 1 < app.config.connections.len() {
                    app.input_field_index += 1;
                }
            }
            KeyCode::Enter => {
                if let Some(conn) = app.config.connections.get(app.input_field_index) {
                    let name = conn.name.clone();
                    let is_ad = conn.is_azure_ad();
                    let ns = conn.namespace.clone().unwrap_or_default();
                    let cs = conn.connection_string.clone().unwrap_or_default();
                    // Drop the borrow on `conn` before mutating `app`
                    let auth_label = if is_ad { "Azure AD" } else { "SAS" };
                    let result = if is_ad {
                        app.connect_azure_ad(&ns)
                    } else {
                        app.connect(&cs)
                    };
                    match result {
                        Ok(_) => {
                            app.connection_name = Some(name);
                            app.modal = ActiveModal::None;
                            app.set_status(format!(
                                "Connected via {}! Loading entities...",
                                auth_label
                            ));
                        }
                        Err(e) => {
                            app.set_error(format!("Connection failed: {}", e));
                            app.modal = ActiveModal::None;
                        }
                    }
                }
            }
            KeyCode::Char('d') => {
                // Delete selected connection
                if let Some(conn) = app.config.connections.get(app.input_field_index) {
                    let name = conn.name.clone();
                    app.config.remove_connection(&name);
                    let _ = app.config.save();
                    if app.input_field_index > 0 {
                        app.input_field_index -= 1;
                    }
                    if app.config.connections.is_empty() {
                        app.modal = ActiveModal::ConnectionModeSelect;
                    }
                }
            }
            _ => {}
        },
        ActiveModal::ConnectionInput => match key.code {
            KeyCode::Esc => {
                app.modal = ActiveModal::None;
            }
            KeyCode::Enter => {
                let cs = app.input_buffer.clone();
                if !cs.is_empty() {
                    match app.connect(&cs) {
                        Ok(_) => {
                            let ns = app
                                .connection_config
                                .as_ref()
                                .map(|c| c.namespace.clone())
                                .unwrap_or_else(|| "default".to_string());
                            app.config.add_connection(ns.clone(), cs);
                            let _ = app.config.save();
                            app.connection_name = Some(ns);
                            app.modal = ActiveModal::None;
                            app.set_status("Connected! Loading entities...");
                        }
                        Err(e) => {
                            app.set_error(format!("Connection failed: {}", e));
                        }
                    }
                }
            }
            KeyCode::Char(c) => {
                app.input_buffer.insert(app.input_cursor, c);
                app.input_cursor += 1;
            }
            KeyCode::Backspace => {
                if app.input_cursor > 0 {
                    app.input_cursor -= 1;
                    app.input_buffer.remove(app.input_cursor);
                }
            }
            KeyCode::Left => {
                if app.input_cursor > 0 {
                    app.input_cursor -= 1;
                }
            }
            KeyCode::Right => {
                if app.input_cursor < app.input_buffer.len() {
                    app.input_cursor += 1;
                }
            }
            KeyCode::Home => app.input_cursor = 0,
            KeyCode::End => app.input_cursor = app.input_buffer.len(),
            _ => {}
        },
        ActiveModal::SendMessage
        | ActiveModal::EditResend
        | ActiveModal::CreateQueue
        | ActiveModal::CreateTopic
        | ActiveModal::CreateSubscription => {
            handle_form_input(app, key);
        }
        ActiveModal::None => {}
    }
}

fn handle_form_input(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.modal = ActiveModal::None;
        }
        _ => {
            handle_field_edit(app, key);
        }
    }
}

/// Inline WYSIWYG edit input handler — Esc exits edit mode (back to read-only detail).
fn handle_detail_edit_input(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.detail_editing = false;
            // Keep selected_message_detail open (back to read-only view)
        }
        _ => {
            handle_field_edit(app, key);
        }
    }
}

/// Shared cursor-based field editing logic for both modal forms and inline edit.
fn handle_field_edit(app: &mut App, key: KeyEvent) {
    let is_body = app.input_field_index == 0
        && app
            .input_fields
            .first()
            .map(|(l, _)| l == "Body")
            .unwrap_or(false);

    match key.code {
        // ── Field navigation (Tab always switches, Up/Down context-dependent) ──
        KeyCode::Tab => {
            if app.input_field_index + 1 < app.input_fields.len() {
                app.input_field_index += 1;
                app.form_cursor = app.input_fields[app.input_field_index].1.len();
            }
        }
        KeyCode::BackTab => {
            if app.input_field_index > 0 {
                app.input_field_index -= 1;
                app.form_cursor = app.input_fields[app.input_field_index].1.len();
            }
        }
        KeyCode::Down if !is_body => {
            if app.input_field_index + 1 < app.input_fields.len() {
                app.input_field_index += 1;
                app.form_cursor = app.input_fields[app.input_field_index].1.len();
            }
        }
        KeyCode::Up if !is_body => {
            if app.input_field_index > 0 {
                app.input_field_index -= 1;
                app.form_cursor = app.input_fields[app.input_field_index].1.len();
            }
        }

        // ── Body multiline: Up/Down move by line ──
        KeyCode::Up if is_body => {
            if let Some((_, ref val)) = app.input_fields.get(app.input_field_index) {
                let (line_start, col) = cursor_line_col(val, app.form_cursor);
                // Find start of previous line
                if line_start > 0 {
                    let prev_line_end = line_start - 1; // the '\n' before current line
                    let prev_text = &val[..prev_line_end];
                    let prev_line_start = prev_text.rfind('\n').map(|i| i + 1).unwrap_or(0);
                    let prev_line_len = prev_line_end - prev_line_start;
                    app.form_cursor = prev_line_start + col.min(prev_line_len);
                }
            }
        }
        KeyCode::Down if is_body => {
            if let Some((_, ref val)) = app.input_fields.get(app.input_field_index) {
                let (line_start, col) = cursor_line_col(val, app.form_cursor);
                // Find start of next line
                if let Some(newline_pos) = val[line_start..].find('\n') {
                    let next_line_start = line_start + newline_pos + 1;
                    let next_line_end = val[next_line_start..]
                        .find('\n')
                        .map(|i| next_line_start + i)
                        .unwrap_or(val.len());
                    let next_line_len = next_line_end - next_line_start;
                    app.form_cursor = next_line_start + col.min(next_line_len);
                }
            }
        }

        // ── Body multiline: Enter inserts newline ──
        KeyCode::Enter if is_body => {
            if let Some((_, ref mut val)) = app.input_fields.get_mut(app.input_field_index) {
                val.insert(app.form_cursor, '\n');
                app.form_cursor += 1;
            }
        }

        // ── Body multiline: Home/End go to start/end of current line ──
        KeyCode::Home if is_body => {
            if let Some((_, ref val)) = app.input_fields.get(app.input_field_index) {
                let (line_start, _) = cursor_line_col(val, app.form_cursor);
                app.form_cursor = line_start;
            }
        }
        KeyCode::End if is_body => {
            if let Some((_, ref val)) = app.input_fields.get(app.input_field_index) {
                let (line_start, _) = cursor_line_col(val, app.form_cursor);
                let line_end = val[line_start..]
                    .find('\n')
                    .map(|i| line_start + i)
                    .unwrap_or(val.len());
                app.form_cursor = line_end;
            }
        }

        // ── Single-line Home/End ──
        KeyCode::Home => {
            app.form_cursor = 0;
        }
        KeyCode::End => {
            if let Some((_, ref val)) = app.input_fields.get(app.input_field_index) {
                app.form_cursor = val.len();
            }
        }

        // ── Submit: F2, Ctrl+Enter, Alt+Enter ──
        KeyCode::F(2) => {
            app.set_status("Submitting...");
        }
        KeyCode::Enter
            if key.modifiers.contains(KeyModifiers::CONTROL)
                || key.modifiers.contains(KeyModifiers::ALT) =>
        {
            app.set_status("Submitting...");
        }

        // ── Cursor navigation ──
        KeyCode::Left => {
            if app.form_cursor > 0 {
                if let Some((_, ref val)) = app.input_fields.get(app.input_field_index) {
                    let prev = val[..app.form_cursor]
                        .char_indices()
                        .next_back()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    app.form_cursor = prev;
                }
            }
        }
        KeyCode::Right => {
            if let Some((_, ref val)) = app.input_fields.get(app.input_field_index) {
                if app.form_cursor < val.len() {
                    let next = val[app.form_cursor..]
                        .char_indices()
                        .nth(1)
                        .map(|(i, _)| app.form_cursor + i)
                        .unwrap_or(val.len());
                    app.form_cursor = next;
                }
            }
        }

        // ── Text editing ──
        KeyCode::Char(c) => {
            if let Some((_, ref mut val)) = app.input_fields.get_mut(app.input_field_index) {
                val.insert(app.form_cursor, c);
                app.form_cursor += c.len_utf8();
            }
        }
        KeyCode::Backspace => {
            if app.form_cursor > 0 {
                if let Some((_, ref mut val)) = app.input_fields.get_mut(app.input_field_index) {
                    let prev = val[..app.form_cursor]
                        .char_indices()
                        .next_back()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    val.drain(prev..app.form_cursor);
                    app.form_cursor = prev;
                }
            }
        }
        KeyCode::Delete => {
            if let Some((_, ref mut val)) = app.input_fields.get_mut(app.input_field_index) {
                if app.form_cursor < val.len() {
                    let next = val[app.form_cursor..]
                        .char_indices()
                        .nth(1)
                        .map(|(i, _)| app.form_cursor + i)
                        .unwrap_or(val.len());
                    val.drain(app.form_cursor..next);
                }
            }
        }
        _ => {}
    }
}

/// Returns (line_start_byte_offset, column_in_bytes) for a cursor position in a multiline string.
fn cursor_line_col(text: &str, cursor: usize) -> (usize, usize) {
    let before = &text[..cursor.min(text.len())];
    let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
    (line_start, cursor - line_start)
}

fn find_parent_topic(app: &App) -> Option<String> {
    // Walk up from selected node to find the topic
    if app.flat_nodes.is_empty() {
        return None;
    }
    let selected = &app.flat_nodes[app.tree_selected];
    // The path for subscriptions is "topic_name/Subscriptions/sub_name"
    if selected.entity_type == EntityType::Subscription {
        return selected.path.split('/').next().map(|s| s.to_string());
    }
    // For subscription folder, look at the parent topic
    if selected.entity_type == EntityType::SubscriptionFolder {
        // Walk backwards to find the topic
        for i in (0..app.tree_selected).rev() {
            if app.flat_nodes[i].entity_type == EntityType::Topic {
                return Some(app.flat_nodes[i].path.clone());
            }
        }
    }
    None
}
