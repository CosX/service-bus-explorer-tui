use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{ActiveModal, App, DiscoveryState};
use crate::client::entity_path;
use crate::client::models::EntityType;

pub fn handle_modal_input(app: &mut App, key: KeyEvent) {
    match &app.modal {
        ActiveModal::Help => {
            app.modal = ActiveModal::None;
        }
        ActiveModal::ConnectionModeSelect => match key.code {
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
        },
        ActiveModal::NamespaceDiscovery { state } => match state {
            DiscoveryState::Loading => {
                if key.code == KeyCode::Esc {
                    app.cancel_bg();
                    app.modal = ActiveModal::None;
                }
            }
            DiscoveryState::List => match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    if app.namespace_list_state > 0 {
                        app.namespace_list_state -= 1;
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if app.namespace_list_state + 1 < app.discovered_namespaces.len() {
                        app.namespace_list_state += 1;
                    }
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
                                let _ = app.config.save();
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
        ActiveModal::CopySelectConnection => match key.code {
            KeyCode::Esc => {
                app.modal = ActiveModal::None;
                app.copy_source_message = None;
                app.copy_source_entity = None;
            }
            KeyCode::Up => {
                if app.input_field_index > 0 {
                    app.input_field_index -= 1;
                    app.copy_connection_list_state
                        .select(Some(app.input_field_index));
                }
            }
            KeyCode::Down => {
                if app.input_field_index + 1 < app.config.connections.len() {
                    app.input_field_index += 1;
                    app.copy_connection_list_state
                        .select(Some(app.input_field_index));
                }
            }
            KeyCode::Char('k') if key.modifiers.is_empty() => {
                if app.input_field_index > 0 {
                    app.input_field_index -= 1;
                    app.copy_connection_list_state
                        .select(Some(app.input_field_index));
                }
            }
            KeyCode::Char('j') if key.modifiers.is_empty() => {
                if app.input_field_index + 1 < app.config.connections.len() {
                    app.input_field_index += 1;
                    app.copy_connection_list_state
                        .select(Some(app.input_field_index));
                }
            }
            KeyCode::Enter => {
                if let Some(conn) = app.config.connections.get(app.input_field_index) {
                    let name = conn.name.clone();
                    let is_ad = conn.is_azure_ad();

                    let config_result: Result<crate::client::ConnectionConfig, String> = if is_ad {
                        if let Some(ref ns) = conn.namespace {
                            match azure_identity::DefaultAzureCredential::new() {
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
                            app.set_status("Loading destination entities...");
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
                if app.copy_entity_selected > 0 {
                    app.copy_entity_selected -= 1;
                    app.copy_entity_list_state
                        .select(Some(app.copy_entity_selected));
                }
            }
            KeyCode::Down => {
                if app.copy_entity_selected + 1 < app.copy_dest_entities.len() {
                    app.copy_entity_selected += 1;
                    app.copy_entity_list_state
                        .select(Some(app.copy_entity_selected));
                }
            }
            KeyCode::Char('k') if key.modifiers.is_empty() => {
                if app.copy_entity_selected > 0 {
                    app.copy_entity_selected -= 1;
                    app.copy_entity_list_state
                        .select(Some(app.copy_entity_selected));
                }
            }
            KeyCode::Char('j') if key.modifiers.is_empty() => {
                if app.copy_entity_selected + 1 < app.copy_dest_entities.len() {
                    app.copy_entity_selected += 1;
                    app.copy_entity_list_state
                        .select(Some(app.copy_entity_selected));
                }
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
                            app.populate_edit_fields(&msg);
                            app.modal = ActiveModal::CopyEditMessage;
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
                        app.populate_edit_fields(&msg);
                        app.modal = ActiveModal::CopyEditMessage;
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
                    let prev_line_len = prev_line_end - prev_line_start;
                    app.form_cursor = prev_line_start + col.min(prev_line_len);
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
                    let next_line_len = next_line_end - next_line_start;
                    app.form_cursor = next_line_start + col.min(next_line_len);
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
            app.set_status("Submitting...");
        }
        KeyCode::Enter
            if key.modifiers.contains(KeyModifiers::CONTROL)
                || key.modifiers.contains(KeyModifiers::ALT) =>
        {
            app.set_status("Submitting...");
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

fn cursor_line_col(text: &str, cursor: usize) -> (usize, usize) {
    let before = &text[..cursor.min(text.len())];
    let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
    (line_start, cursor - line_start)
}
