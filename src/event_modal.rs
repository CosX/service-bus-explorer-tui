use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{
    ActiveModal, App, ClearAction, ClearScope, Command, DiscoveryState, PendingClear,
};
use crate::client::entity_path;
use crate::client::models::EntityType;
use crate::ui::sanitize::floor_char_boundary;

/// Queue the confirmed clear operation for the main loop.
fn start_clear(app: &mut App, scope: ClearScope, action: ClearAction) {
    app.request(
        Command::Clear(PendingClear { scope, action }),
        action.status_text(),
    );
}

fn move_selection_up(selected: &mut usize) {
    if *selected > 0 {
        *selected -= 1;
    }
}

fn move_selection_down(selected: &mut usize, len: usize) {
    if *selected + 1 < len {
        *selected += 1;
    }
}

fn handle_single_line_input(
    input: &mut String,
    cursor: &mut usize,
    key: KeyEvent,
    allow_char: impl Fn(char) -> bool,
) -> bool {
    match key.code {
        KeyCode::Char(c) if allow_char(c) => {
            input.insert(*cursor, c);
            *cursor += 1;
            true
        }
        KeyCode::Backspace => {
            if *cursor > 0 {
                *cursor -= 1;
                input.remove(*cursor);
            }
            true
        }
        KeyCode::Left => {
            if *cursor > 0 {
                *cursor -= 1;
            }
            true
        }
        KeyCode::Right => {
            if *cursor < input.len() {
                *cursor += 1;
            }
            true
        }
        KeyCode::Home => {
            *cursor = 0;
            true
        }
        KeyCode::End => {
            *cursor = input.len();
            true
        }
        _ => false,
    }
}

pub fn handle_modal_input(app: &mut App, key: KeyEvent) {
    match &app.modal {
        // Navigation keys (and the mouse wheel) scroll; anything else closes.
        ActiveModal::Help => {
            const PAGE: u16 = 10;
            let max = app.help_max_scroll;
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    app.help_scroll = app.help_scroll.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    app.help_scroll = app.help_scroll.saturating_add(1).min(max);
                }
                KeyCode::PageUp => app.help_scroll = app.help_scroll.saturating_sub(PAGE),
                KeyCode::PageDown => {
                    app.help_scroll = app.help_scroll.saturating_add(PAGE).min(max);
                }
                KeyCode::Home | KeyCode::Char('g') => app.help_scroll = 0,
                KeyCode::End | KeyCode::Char('G') => app.help_scroll = max,
                _ => app.modal = ActiveModal::None,
            }
        }
        ActiveModal::MetricsDetail => {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('V') => {
                    app.modal = ActiveModal::None;
                }
                // Allow cycling the time window while the overlay is open
                KeyCode::Char('M') if app.metrics.is_available() => {
                    app.metrics_window = app.metrics_window.next();
                    app.entity_metrics = None;
                    app.metrics_failure = None;
                    app.set_status(format!("Metrics window: {}", app.metrics_window.label()));
                }
                _ => {}
            }
        }
        ActiveModal::ConnectionModeSelect => {
            match key.code {
                KeyCode::Char('1') | KeyCode::Char('s') | KeyCode::Char('S') => {
                    app.input_buffer.clear();
                    app.input_cursor = 0;
                    app.modal = ActiveModal::ConnectionInput;
                }
                KeyCode::Char('2') | KeyCode::Char('a') | KeyCode::Char('A') => {
                    app.start_namespace_discovery();
                }
                KeyCode::Esc => {
                    app.modal = ActiveModal::None;
                }
                _ => {}
            }
            return;
        }
        ActiveModal::NamespaceDiscovery { state } => match state {
            DiscoveryState::Loading => {
                if key.code == KeyCode::Esc {
                    app.cancel_bg();
                    app.modal = ActiveModal::None;
                }
            }
            DiscoveryState::List => match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    move_selection_up(&mut app.namespace_list_state);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    move_selection_down(
                        &mut app.namespace_list_state,
                        app.discovered_namespaces.len(),
                    );
                }
                KeyCode::Enter => {
                    if let Some(ns) = app
                        .discovered_namespaces
                        .get(app.namespace_list_state)
                        .cloned()
                    {
                        match app.connect_azure_ad(&ns.fqdn) {
                            Ok(_) => {
                                app.config
                                    .add_azure_ad_connection(ns.name.clone(), ns.fqdn.clone());
                                app.save_config();
                                app.connection_name = Some(ns.name.clone());
                                app.modal = ActiveModal::None;
                                app.set_status("Connected via Azure AD! Loading entities...");
                            }
                            Err(e) => {
                                app.set_error(format!("Azure AD connection failed: {}", e));
                                app.modal = ActiveModal::None;
                            }
                        }
                    }
                }
                KeyCode::Char('m') | KeyCode::Char('M') => {
                    app.input_buffer.clear();
                    app.input_cursor = 0;
                    app.modal = ActiveModal::AzureAdNamespaceInput;
                }
                KeyCode::Esc => {
                    app.modal = ActiveModal::None;
                }
                _ => {}
            },
            DiscoveryState::Error(_) => match key.code {
                KeyCode::Char('m') | KeyCode::Char('M') => {
                    app.input_buffer.clear();
                    app.input_cursor = 0;
                    app.modal = ActiveModal::AzureAdNamespaceInput;
                }
                KeyCode::Esc => {
                    app.modal = ActiveModal::None;
                }
                _ => {}
            },
        },
        ActiveModal::AzureAdNamespaceInput => match key.code {
            KeyCode::Enter => {
                let ns = app.input_buffer.trim().to_string();
                if !ns.is_empty() {
                    let fqns = if ns.contains('.') {
                        ns.clone()
                    } else {
                        format!("{}.servicebus.windows.net", ns)
                    };
                    match app.connect_azure_ad(&fqns) {
                        Ok(_) => {
                            app.config
                                .add_azure_ad_connection(fqns.clone(), fqns.clone());
                            app.save_config();
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
            _ => {}
        },
        ActiveModal::ConfirmDelete(ref path) => match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                let path = path.clone();
                app.request(Command::DeleteEntity(path), "Deleting...");
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                app.modal = ActiveModal::None;
            }
            _ => {}
        },
        ActiveModal::ConfirmBulkResend { .. } => match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                app.request(Command::BulkResend, "Resending...");
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                app.modal = ActiveModal::None;
            }
            _ => {}
        },
        ActiveModal::ConfirmBulkDelete { .. } => match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                app.request(Command::BulkDelete, "Deleting messages...");
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                app.modal = ActiveModal::None;
            }
            _ => {}
        },
        ActiveModal::ConfirmSingleDelete { .. } => match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                app.request(Command::DeleteMessage, "Deleting message...");
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                app.modal = ActiveModal::None;
            }
            _ => {}
        },
        ActiveModal::PeekCountInput => match key.code {
            KeyCode::Enter => {
                if let Ok(count) = app.input_buffer.trim().parse::<usize>() {
                    if count > 0 {
                        app.modal = ActiveModal::None;
                        let dlq = std::mem::take(&mut app.peek_dlq);
                        app.request(Command::Peek { count, dlq }, "Peeking messages...");
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
            _ => {}
        },
        ActiveModal::ConfirmPurgeAllDlq => match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                app.request(Command::PurgeAllDlq, "Purging all DLQ...");
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                app.modal = ActiveModal::None;
            }
            _ => {}
        },
        ActiveModal::ClearOptions { ref scope } => {
            let action = match key.code {
                KeyCode::Char('d') | KeyCode::Char('D') => Some(ClearAction::DeleteActive),
                KeyCode::Char('l') | KeyCode::Char('L') => Some(ClearAction::DeleteDlq),
                KeyCode::Char('r') | KeyCode::Char('R') => Some(ClearAction::ResendDlq),
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                    app.modal = ActiveModal::None;
                    return;
                }
                _ => None,
            };
            if let Some(action) = action {
                let scope = scope.clone();
                if scope.is_batch() {
                    // Namespace-wide clears get an extra confirmation step.
                    app.modal = ActiveModal::ConfirmClearBatch { scope, action };
                } else {
                    start_clear(app, scope, action);
                }
            }
        }
        ActiveModal::ConfirmClearBatch { ref scope, action } => match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                let (scope, action) = (scope.clone(), *action);
                start_clear(app, scope, action);
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
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
                move_selection_up(&mut app.input_field_index);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                move_selection_down(&mut app.input_field_index, app.config.connections.len());
            }
            KeyCode::Enter => {
                if let Some(conn) = app.config.connections.get(app.input_field_index) {
                    let name = conn.name.clone();
                    let is_ad = conn.is_azure_ad();
                    let ns = conn.namespace.clone().unwrap_or_default();
                    let cs = conn.connection_string.clone().unwrap_or_default();
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
                if let Some(conn) = app.config.connections.get(app.input_field_index) {
                    let name = conn.name.clone();
                    app.config.remove_connection(&name);
                    app.save_config();
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
        ActiveModal::ConnectionSwitch => match key.code {
            KeyCode::Char('d') | KeyCode::Char('D') => {
                app.disconnect();
                app.modal = ActiveModal::None;
                app.set_status("Disconnected");
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                app.disconnect();
                app.input_buffer.clear();
                app.input_cursor = 0;
                app.input_field_index = 0;
                if app.config.connections.is_empty() {
                    app.modal = ActiveModal::ConnectionModeSelect;
                } else {
                    app.modal = ActiveModal::ConnectionList;
                }
            }
            KeyCode::Esc | KeyCode::Char('c') | KeyCode::Char('C') => {
                app.modal = ActiveModal::None;
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
                            app.save_config();
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
            _ => {}
        },
        ActiveModal::CopySelectConnection => match key.code {
            KeyCode::Esc => {
                app.modal = ActiveModal::None;
                app.copy_source_message = None;
                app.copy_source_entity = None;
            }
            KeyCode::Up => {
                move_selection_up(&mut app.input_field_index);
                app.copy_connection_list_state
                    .select(Some(app.input_field_index));
            }
            KeyCode::Down => {
                move_selection_down(&mut app.input_field_index, app.config.connections.len());
                app.copy_connection_list_state
                    .select(Some(app.input_field_index));
            }
            KeyCode::Char('k') if key.modifiers.is_empty() => {
                move_selection_up(&mut app.input_field_index);
                app.copy_connection_list_state
                    .select(Some(app.input_field_index));
            }
            KeyCode::Char('j') if key.modifiers.is_empty() => {
                move_selection_down(&mut app.input_field_index, app.config.connections.len());
                app.copy_connection_list_state
                    .select(Some(app.input_field_index));
            }
            KeyCode::Enter => {
                if let Some(conn) = app.config.connections.get(app.input_field_index) {
                    let name = conn.name.clone();
                    let is_ad = conn.is_azure_ad();

                    let config_result: Result<crate::client::ConnectionConfig, String> = if is_ad {
                        if let Some(ref ns) = conn.namespace {
                            match azure_identity::DeveloperToolsCredential::new(None) {
                                Ok(cred) => {
                                    Ok(crate::client::ConnectionConfig::from_azure_ad(ns, cred))
                                }
                                Err(e) => Err(format!("Azure AD credential error: {}", e)),
                            }
                        } else {
                            Err("No namespace configured for Azure AD connection".to_string())
                        }
                    } else if let Some(ref cs) = conn.connection_string {
                        crate::client::ConnectionConfig::from_connection_string(cs)
                            .map_err(|e| format!("Connection string parse error: {}", e))
                    } else {
                        Err("No connection string configured".to_string())
                    };

                    match config_result {
                        Ok(config) => {
                            app.copy_dest_connection_name = Some(name);
                            app.copy_dest_connection_config = Some(config);
                            app.copy_dest_entities.clear();
                            app.copy_entity_selected = 0;
                            app.copy_entity_list_state.select(Some(0));
                            app.request(
                                Command::LoadDestinationEntities,
                                "Loading destination entities...",
                            );
                            app.modal = ActiveModal::CopySelectEntity;
                        }
                        Err(e) => {
                            app.set_error(format!(
                                "Failed to create destination connection: {}",
                                e
                            ));
                            app.modal = ActiveModal::None;
                        }
                    }
                }
            }
            _ => {}
        },
        ActiveModal::CopySelectEntity => match key.code {
            KeyCode::Esc => {
                app.modal = ActiveModal::None;
                app.copy_source_message = None;
                app.copy_source_entity = None;
                app.copy_dest_entities.clear();
                app.copy_entity_selected = 0;
                app.copy_dest_connection_name = None;
                app.copy_dest_connection_config = None;
            }
            KeyCode::Up => {
                move_selection_up(&mut app.copy_entity_selected);
                app.copy_entity_list_state
                    .select(Some(app.copy_entity_selected));
            }
            KeyCode::Down => {
                move_selection_down(&mut app.copy_entity_selected, app.copy_dest_entities.len());
                app.copy_entity_list_state
                    .select(Some(app.copy_entity_selected));
            }
            KeyCode::Char('k') if key.modifiers.is_empty() => {
                move_selection_up(&mut app.copy_entity_selected);
                app.copy_entity_list_state
                    .select(Some(app.copy_entity_selected));
            }
            KeyCode::Char('j') if key.modifiers.is_empty() => {
                move_selection_down(&mut app.copy_entity_selected, app.copy_dest_entities.len());
                app.copy_entity_list_state
                    .select(Some(app.copy_entity_selected));
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                if let Some(ref src_entity) = app.copy_source_entity {
                    let entity_name = entity_path::send_target(src_entity);
                    let exists = app
                        .copy_dest_entities
                        .iter()
                        .any(|(name, _)| name == entity_name);

                    if exists {
                        app.copy_destination_entity = Some(entity_name.to_string());
                        if let Some(msg) = app.copy_source_message.clone() {
                            match app.populate_edit_fields(&msg) {
                                Ok(()) => app.modal = ActiveModal::CopyEditMessage,
                                Err(e) => app.set_error(e),
                            }
                        }
                    } else {
                        app.set_error(format!("Entity '{}' not found in destination", entity_name));
                    }
                }
            }
            KeyCode::Enter => {
                if let Some((entity, _)) = app.copy_dest_entities.get(app.copy_entity_selected) {
                    app.copy_destination_entity = Some(entity.clone());
                    if let Some(msg) = app.copy_source_message.clone() {
                        match app.populate_edit_fields(&msg) {
                            Ok(()) => app.modal = ActiveModal::CopyEditMessage,
                            Err(e) => app.set_error(e),
                        }
                    }
                }
            }
            _ => {}
        },
        ActiveModal::SendMessage
        | ActiveModal::EditResend
        | ActiveModal::CreateQueue
        | ActiveModal::CreateTopic
        | ActiveModal::CreateSubscription
        | ActiveModal::EditSubscriptionFilter
        | ActiveModal::CopyEditMessage => {
            handle_form_input(app, key);
        }
        ActiveModal::None => {}
    }

    match &app.modal {
        ActiveModal::AzureAdNamespaceInput => {
            let _ =
                handle_single_line_input(&mut app.input_buffer, &mut app.input_cursor, key, |_| {
                    true
                });
        }
        ActiveModal::PeekCountInput => {
            let _ =
                handle_single_line_input(&mut app.input_buffer, &mut app.input_cursor, key, |c| {
                    c.is_ascii_digit()
                });
        }
        ActiveModal::ConnectionInput => {
            let _ =
                handle_single_line_input(&mut app.input_buffer, &mut app.input_cursor, key, |_| {
                    true
                });
        }
        _ => {}
    }
}

pub fn find_parent_topic(app: &App) -> Option<String> {
    if app.flat_nodes.is_empty() {
        return None;
    }

    let selected = &app.flat_nodes[app.tree_selected];
    if selected.entity_type == EntityType::Subscription {
        return Some(entity_path::send_target(&selected.path).to_string());
    }

    if selected.entity_type == EntityType::SubscriptionFolder {
        for i in (0..app.tree_selected).rev() {
            if app.flat_nodes[i].entity_type == EntityType::Topic {
                return Some(app.flat_nodes[i].path.clone());
            }
        }
    }

    None
}

pub fn handle_detail_edit_input(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.detail_editing = false;
        }
        _ => {
            handle_field_edit(app, key);
        }
    }
}

fn handle_form_input(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            if app.modal == ActiveModal::CopyEditMessage {
                app.copy_source_message = None;
                app.copy_source_entity = None;
                app.copy_dest_entities.clear();
                app.copy_entity_selected = 0;
                app.copy_dest_connection_name = None;
                app.copy_dest_connection_config = None;
                app.copy_destination_entity = None;
            }
            app.modal = ActiveModal::None;
        }
        _ => {
            handle_field_edit(app, key);
        }
    }
}

fn handle_field_edit(app: &mut App, key: KeyEvent) {
    // Every edit below slices the field at the cursor; keep it on a char
    // boundary so multi-byte text (æ, ø, emoji) can never cause a panic.
    if let Some((_, ref val)) = app.input_fields.get(app.input_field_index) {
        app.form_cursor = floor_char_boundary(val, app.form_cursor);
    }
    let is_body = app.input_field_index == 0
        && app
            .input_fields
            .first()
            .map(|(l, _)| l == "Body")
            .unwrap_or(false);

    match key.code {
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
        KeyCode::Up if is_body => {
            if let Some((_, ref val)) = app.input_fields.get(app.input_field_index) {
                let (line_start, col) = cursor_line_col(val, app.form_cursor);
                if line_start > 0 {
                    let prev_line_end = line_start - 1;
                    let prev_text = &val[..prev_line_end];
                    let prev_line_start = prev_text.rfind('\n').map(|i| i + 1).unwrap_or(0);
                    app.form_cursor = offset_at_col(val, prev_line_start, prev_line_end, col);
                }
            }
        }
        KeyCode::Down if is_body => {
            if let Some((_, ref val)) = app.input_fields.get(app.input_field_index) {
                let (line_start, col) = cursor_line_col(val, app.form_cursor);
                if let Some(newline_pos) = val[line_start..].find('\n') {
                    let next_line_start = line_start + newline_pos + 1;
                    let next_line_end = val[next_line_start..]
                        .find('\n')
                        .map(|i| next_line_start + i)
                        .unwrap_or(val.len());
                    app.form_cursor = offset_at_col(val, next_line_start, next_line_end, col);
                }
            }
        }
        KeyCode::Enter if is_body => {
            if let Some((_, ref mut val)) = app.input_fields.get_mut(app.input_field_index) {
                val.insert(app.form_cursor, '\n');
                app.form_cursor += 1;
            }
        }
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
        KeyCode::Home => {
            app.form_cursor = 0;
        }
        KeyCode::End => {
            if let Some((_, ref val)) = app.input_fields.get(app.input_field_index) {
                app.form_cursor = val.len();
            }
        }
        KeyCode::F(2) => {
            app.request(Command::SubmitForm, "Submitting...");
        }
        KeyCode::Enter
            if key.modifiers.contains(KeyModifiers::CONTROL)
                || key.modifiers.contains(KeyModifiers::ALT) =>
        {
            app.request(Command::SubmitForm, "Submitting...");
        }
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

/// Byte offset of the cursor's line start, and the cursor's column in chars.
fn cursor_line_col(text: &str, cursor: usize) -> (usize, usize) {
    let cursor = floor_char_boundary(text, cursor);
    let before = &text[..cursor];
    let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
    (line_start, text[line_start..cursor].chars().count())
}

/// Byte offset of char column `col` within the line `text[start..end]`,
/// clamped to the line end. Always a char boundary.
fn offset_at_col(text: &str, start: usize, end: usize, col: usize) -> usize {
    text[start..end]
        .char_indices()
        .nth(col)
        .map(|(i, _)| start + i)
        .unwrap_or(end)
}

#[cfg(test)]
mod cursor_tests {
    use super::{cursor_line_col, offset_at_col};

    #[test]
    fn column_is_counted_in_chars_not_bytes() {
        let text = "æøå\nabc";
        // Cursor after "ab" on line 2.
        let (start, col) = cursor_line_col(text, text.len() - 1);
        assert_eq!(start, "æøå\n".len());
        assert_eq!(col, 2);
    }

    #[test]
    fn moving_up_into_multibyte_text_lands_on_a_char_boundary() {
        let text = "æøå\nabc";
        let (_, col) = cursor_line_col(text, text.len() - 1); // col 2
        let up = offset_at_col(text, 0, "æøå".len(), col);
        assert!(text.is_char_boundary(up));
        assert_eq!(&text[..up], "æø");
    }

    #[test]
    fn column_past_line_end_clamps_to_the_end() {
        let text = "ab\nlonger line";
        assert_eq!(offset_at_col(text, 0, 2, 9), 2);
    }
}
