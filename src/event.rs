use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::time::Duration;

use crate::app::{ActiveModal, App, FocusPanel, MessageTab};
use crate::client::models::EntityType;
use crate::event_modal;

const BG_BUSY_MSG: &str = "A background operation is in progress...";

/// Poll for input events and process them against app state.
/// Returns true if the app should continue running.
pub fn handle_events(app: &mut App) -> anyhow::Result<bool> {
    if event::poll(Duration::from_millis(100))? {
        if let Event::Key(key) = event::read()? {
            // On Windows, crossterm emits both Press and Release events.
            // Only handle Press to avoid processing each keystroke twice.
            if key.kind != KeyEventKind::Press {
                return Ok(app.running);
            }

            // If a background operation is running, Esc cancels it
            if app.bg_running && key.code == KeyCode::Esc {
                app.cancel_bg();
                app.set_status("Cancelling...");
                return Ok(app.running);
            }

            // If a modal is open, route to modal handler
            if app.modal != ActiveModal::None {
                event_modal::handle_modal_input(app, key);
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
                    if app.bg_running {
                        app.set_status(
                            "A background operation is in progress. Press Esc to cancel first.",
                        );
                    } else if app.management.is_none() {
                        // Open connection flow
                        app.input_buffer.clear();
                        app.input_cursor = 0;
                        if app.config.connections.is_empty() {
                            app.modal = ActiveModal::ConnectionModeSelect;
                        } else {
                            app.modal = ActiveModal::ConnectionList;
                        }
                    } else {
                        // Already connected — open switch modal
                        app.modal = ActiveModal::ConnectionSwitch;
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
            move_selection_up(&mut app.tree_selected);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            move_selection_down(&mut app.tree_selected, app.flat_nodes.len());
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
            if !block_if_bg_running(app, BG_BUSY_MSG) {
                app.last_refresh = Some(std::time::Instant::now());
                app.set_status("Refreshing...");
                // Trigger async refresh — handled in main loop
            }
        }
        // 't' = toggle auto-refresh timer
        KeyCode::Char('t') => {
            app.auto_refresh_enabled = !app.auto_refresh_enabled;
            if app.auto_refresh_enabled {
                if app.config.settings.auto_refresh_secs == 0 {
                    app.config.settings.auto_refresh_secs =
                        crate::config::default_auto_refresh_secs();
                }
                app.last_refresh = Some(std::time::Instant::now());
                app.set_status(format!(
                    "Auto-refresh enabled (every {}s)",
                    app.config.settings.auto_refresh_secs
                ));
            } else {
                app.set_status("Auto-refresh disabled");
            }
        }
        // 's' = send message to selected entity
        KeyCode::Char('s') => {
            if !block_if_bg_running(app, BG_BUSY_MSG) {
                if let Some((_, entity_type)) = app.selected_entity() {
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
        }
        // 'p' = peek messages — prompt for count
        KeyCode::Char('p') => {
            if !block_if_bg_running(app, BG_BUSY_MSG) {
                if let Some((_, entity_type)) = app.selected_entity() {
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
        }
        // 'n' = new entity
        KeyCode::Char('n') => {
            if !block_if_bg_running(app, BG_BUSY_MSG) && !app.flat_nodes.is_empty() {
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
                        let topic = event_modal::find_parent_topic(app);
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
            if !block_if_bg_running(app, BG_BUSY_MSG) {
                if let Some((
                    path,
                    entity_type
                    @ (EntityType::Queue | EntityType::Topic | EntityType::Subscription),
                )) = app.selected_entity()
                {
                    let _ = entity_type;
                    let path = path.to_string();
                    app.modal = ActiveModal::ConfirmDelete(path);
                    app.input_buffer.clear();
                }
            }
        }
        // 'd' = peek dead-letter queue for selected entity
        KeyCode::Char('d') => {
            if !block_if_bg_running(app, BG_BUSY_MSG) {
                if let Some((_, entity_type)) = app.selected_entity() {
                    match entity_type {
                        EntityType::Queue | EntityType::Subscription | EntityType::Topic => {
                            app.input_buffer = app.config.settings.peek_count.to_string();
                            app.input_cursor = app.input_buffer.len();
                            app.modal = ActiveModal::PeekCountInput;
                            app.peek_dlq = true;
                        }
                        _ => {
                            app.set_status(
                                "Select a queue, topic, or subscription to peek its DLQ",
                            );
                        }
                    }
                }
            }
        }
        // 'P' (shift+p) = clear entity (choose delete or resend)
        KeyCode::Char('P') => {
            if !block_if_bg_running(app, BG_BUSY_MSG) {
                if let Some((path, entity_type)) = app.selected_entity() {
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
        }
        // 'f' = edit subscription SQL filter rule
        KeyCode::Char('f') => {
            if !block_if_bg_running(app, BG_BUSY_MSG) {
                if let Some((_, entity_type)) = app.selected_entity() {
                    if *entity_type == EntityType::Subscription {
                        app.set_status("Loading subscription filters...");
                    } else {
                        app.set_status("Select a subscription to edit its filter");
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
        event_modal::handle_detail_edit_input(app, key);
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
            } else {
                move_selection_up(&mut app.message_selected);
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.selected_message_detail.is_some() {
                app.detail_body_scroll = app.detail_body_scroll.saturating_add(1);
            } else {
                move_selection_down(&mut app.message_selected, len);
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
            if block_if_bg_running(app, BG_BUSY_MSG) {
                return;
            }
            if app.message_tab == MessageTab::DeadLetter {
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
            if !block_if_bg_running(app, BG_BUSY_MSG) {
                if let Some((path, entity_type)) = app.selected_entity() {
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
        }
        // x = Delete single selected message
        KeyCode::Char('x') => {
            if !block_if_bg_running(app, BG_BUSY_MSG) {
                let is_dlq = app.message_tab == MessageTab::DeadLetter;
                let msg = if app.selected_message_detail.is_some() {
                    app.selected_message_detail.clone()
                } else {
                    let msgs = if is_dlq {
                        &app.dlq_messages
                    } else {
                        &app.messages
                    };
                    msgs.get(app.message_selected).cloned()
                };
                if let Some(msg) = msg {
                    if let Some(seq) = msg.broker_properties.sequence_number {
                        if let Some((path, _)) = app.selected_entity() {
                            let entity_path = if is_dlq {
                                msg.source_entity
                                    .clone()
                                    .unwrap_or_else(|| path.to_string())
                            } else {
                                path.to_string()
                            };
                            app.modal = ActiveModal::ConfirmSingleDelete {
                                entity_path,
                                sequence_number: seq,
                                is_dlq,
                            };
                        }
                    } else {
                        app.set_status("Message has no sequence number");
                    }
                } else {
                    app.set_status("No message selected");
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
        // C = Copy message to different connection/entity
        KeyCode::Char('C') => {
            if !block_if_bg_running(app, BG_BUSY_MSG) {
                // Clone all necessary data before any mutations
                let msg = if app.selected_message_detail.is_some() {
                    app.selected_message_detail.clone()
                } else {
                    match app.message_tab {
                        MessageTab::Messages => app.messages.get(app.message_selected).cloned(),
                        MessageTab::DeadLetter => {
                            app.dlq_messages.get(app.message_selected).cloned()
                        }
                    }
                };
                let has_connections = !app.config.connections.is_empty();
                let entity_path = app.selected_entity().map(|(path, _)| path.to_string());

                if let Some(message) = msg {
                    if !has_connections {
                        app.set_error("No saved connections available. Add a connection first.");
                    } else if let Some(path) = entity_path {
                        app.copy_source_message = Some(message);
                        app.copy_source_entity = Some(path);
                        app.input_field_index = 0;
                        app.copy_connection_list_state.select(Some(0));
                        app.modal = ActiveModal::CopySelectConnection;
                    }
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

fn block_if_bg_running(app: &mut App, message: &str) -> bool {
    if app.bg_running {
        app.set_status(message);
        true
    } else {
        false
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use std::time::Instant;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn new_app_on_tree() -> App {
        let mut app = App::new();
        app.focus = FocusPanel::Tree;
        app
    }

    #[test]
    fn toggle_auto_refresh_enables_when_disabled() {
        let mut app = new_app_on_tree();
        app.auto_refresh_enabled = false;
        app.config.settings.auto_refresh_secs = 30;
        app.last_refresh = None;

        handle_tree_input(&mut app, press(KeyCode::Char('t')));

        assert!(app.auto_refresh_enabled);
        assert!(app.last_refresh.is_some());
        assert!(app.status_message.contains("Auto-refresh enabled"));
        assert!(app.status_message.contains("30"));
    }

    #[test]
    fn toggle_auto_refresh_disables_when_enabled() {
        let mut app = new_app_on_tree();
        app.auto_refresh_enabled = true;
        app.config.settings.auto_refresh_secs = 30;
        app.last_refresh = Some(Instant::now());

        handle_tree_input(&mut app, press(KeyCode::Char('t')));

        assert!(!app.auto_refresh_enabled);
        assert_eq!(app.status_message, "Auto-refresh disabled");
    }

    #[test]
    fn toggle_auto_refresh_roundtrip() {
        let mut app = new_app_on_tree();
        app.auto_refresh_enabled = false;
        app.config.settings.auto_refresh_secs = 30;

        // Enable
        handle_tree_input(&mut app, press(KeyCode::Char('t')));
        assert!(app.auto_refresh_enabled);

        // Disable
        handle_tree_input(&mut app, press(KeyCode::Char('t')));
        assert!(!app.auto_refresh_enabled);
    }

    #[test]
    fn toggle_auto_refresh_resets_last_refresh_on_enable() {
        let mut app = new_app_on_tree();
        app.auto_refresh_enabled = false;
        app.config.settings.auto_refresh_secs = 30;
        // Simulate stale last_refresh from a previous enable/disable cycle.
        // Use checked_sub to avoid overflow on Windows where Instant can't
        // go below the system boot time (fresh CI runners may have < 999s uptime).
        app.last_refresh = Some(
            Instant::now()
                .checked_sub(std::time::Duration::from_secs(999))
                .unwrap_or(Instant::now()),
        );

        handle_tree_input(&mut app, press(KeyCode::Char('t')));

        let last = app.last_refresh.unwrap();
        // last_refresh should be very recent (within 1s)
        assert!(last.elapsed().as_secs() < 1);
    }

    #[test]
    fn toggle_auto_refresh_sets_default_when_secs_zero() {
        let mut app = new_app_on_tree();
        app.auto_refresh_enabled = false;
        app.config.settings.auto_refresh_secs = 0;

        handle_tree_input(&mut app, press(KeyCode::Char('t')));

        assert!(app.auto_refresh_enabled);
        assert_eq!(
            app.config.settings.auto_refresh_secs,
            crate::config::default_auto_refresh_secs()
        );
        assert!(app.status_message.contains("Auto-refresh enabled"));
        assert!(app.last_refresh.is_some());
    }

    #[test]
    fn manual_refresh_sets_last_refresh() {
        let mut app = new_app_on_tree();
        app.last_refresh = None;

        handle_tree_input(&mut app, press(KeyCode::Char('r')));

        assert!(app.last_refresh.is_some());
        assert_eq!(app.status_message, "Refreshing...");
    }

    #[test]
    fn manual_refresh_blocked_while_bg_running() {
        let mut app = new_app_on_tree();
        app.bg_running = true;
        app.last_refresh = None;

        handle_tree_input(&mut app, press(KeyCode::Char('r')));

        // last_refresh should NOT be set when blocked
        assert!(app.last_refresh.is_none());
        assert_eq!(app.status_message, BG_BUSY_MSG);
    }
}
