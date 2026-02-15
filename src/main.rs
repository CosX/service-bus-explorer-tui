mod app;
mod client;
mod config;
mod event;
mod ui;

use std::io;

use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;

use app::{ActiveModal, App, BgEvent, DetailView, FocusPanel, MessageTab};
use client::models::EntityType;

/// Resolve an entity path to the path suitable for sending messages.
/// Subscriptions ("topic/Subscriptions/sub") → topic name.
/// Queues remain unchanged.
fn send_path(entity_path: &str) -> &str {
    // Subscription paths contain "/Subscriptions/" (case-insensitive match)
    if let Some(idx) = entity_path.find("/Subscriptions/").or_else(|| entity_path.find("/subscriptions/")) {
        &entity_path[..idx]
    } else {
        entity_path
    }
}

/// Owned version of `send_path` for use in spawned tasks.
fn send_path_owned(entity_path: &str) -> String {
    send_path(entity_path).to_string()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(e) = result {
        eprintln!("Error: {}", e);
    }

    Ok(())
}

async fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> anyhow::Result<()> {
    let mut app = App::new();
    let mut needs_refresh = false;
    let mut last_selected: usize = usize::MAX;

    loop {
        // Draw
        terminal.draw(|frame| {
            ui::layout::render(frame, &mut app);
        })?;

        // Handle events
        if !event::handle_events(&mut app)? {
            break;
        }

        if !app.running {
            break;
        }

        // ──────── Poll background task results ────────
        while let Ok(event) = app.bg_rx.try_recv() {
            match event {
                BgEvent::Progress(msg) => {
                    app.set_status(msg);
                }
                BgEvent::PurgeComplete { count } => {
                    app.set_status(format!("Deleted {} messages", count));
                    app.messages.clear();
                    app.dlq_messages.clear();
                    app.message_selected = 0;
                    app.bg_running = false;
                    needs_refresh = true;
                }
                BgEvent::ResendComplete { resent, errors } => {
                    if errors > 0 {
                        app.set_status(format!(
                            "Resent {} messages ({} errors)",
                            resent, errors
                        ));
                    } else {
                        app.set_status(format!("Resent {} messages", resent));
                    }
                    app.dlq_messages.clear();
                    app.message_selected = 0;
                    app.bg_running = false;
                    needs_refresh = true;
                }
                BgEvent::BulkDeleteComplete { deleted, was_dlq } => {
                    app.set_status(format!("Deleted {} messages", deleted));
                    if was_dlq {
                        app.dlq_messages.clear();
                    } else {
                        app.messages.clear();
                    }
                    app.message_selected = 0;
                    app.bg_running = false;
                    needs_refresh = true;
                }
                BgEvent::Cancelled { message } => {
                    app.set_status(message);
                    app.bg_running = false;
                    needs_refresh = true;
                }
                BgEvent::Failed(msg) => {
                    app.set_error(msg);
                    app.bg_running = false;
                    app.loading = false;
                }
                BgEvent::TreeRefreshed { tree, flat_nodes } => {
                    let q_count = flat_nodes
                        .iter()
                        .filter(|n| n.entity_type == EntityType::Queue)
                        .count();
                    let t_count = flat_nodes
                        .iter()
                        .filter(|n| n.entity_type == EntityType::Topic)
                        .count();
                    app.flat_nodes = flat_nodes;
                    app.tree = Some(tree);
                    if app.tree_selected >= app.flat_nodes.len() {
                        app.tree_selected = 0;
                    }
                    app.loading = false;
                    app.set_status(format!("Loaded {} queues, {} topics", q_count, t_count));
                }
                BgEvent::DetailLoaded(detail) => {
                    app.detail_view = detail;
                }
                BgEvent::PeekComplete { messages, is_dlq } => {
                    let count = messages.len();
                    if is_dlq {
                        app.dlq_messages = messages;
                        app.message_tab = MessageTab::DeadLetter;
                    } else {
                        app.messages = messages;
                        app.message_tab = MessageTab::Messages;
                    }
                    app.message_selected = 0;
                    app.selected_message_detail = None;
                    app.focus = FocusPanel::Messages;
                    if is_dlq {
                        app.set_status(format!("Peeked {} DLQ messages", count));
                    } else {
                        app.set_status(format!("Peeked {} messages", count));
                    }
                }
                BgEvent::SendComplete { status } => {
                    app.set_status(status);
                    app.modal = ActiveModal::None;
                }
                BgEvent::EntityCreated { status } => {
                    app.set_status(status);
                    app.modal = ActiveModal::None;
                    needs_refresh = true;
                }
                BgEvent::EntityDeleted { status } => {
                    app.set_status(status);
                    app.modal = ActiveModal::None;
                    needs_refresh = true;
                }
                BgEvent::ResendSendComplete { status, dlq_seq_removed, was_inline } => {
                    if let Some(seq) = dlq_seq_removed {
                        app.dlq_messages.retain(|m| {
                            m.broker_properties.sequence_number != Some(seq)
                        });
                    }
                    app.set_status(status);
                    if was_inline {
                        app.detail_editing = false;
                        app.selected_message_detail = None;
                    } else {
                        app.modal = ActiveModal::None;
                    }
                }
            }
        }

        // ──────── Async action dispatch ────────
        // All operations are spawned as background tasks to keep the UI responsive.

        // Connection just established — trigger tree refresh
        if app.management.is_some() && app.tree.is_none() && !app.loading {
            needs_refresh = true;
        }

        // Refresh tree (spawned)
        if needs_refresh || app.status_message == "Refreshing..." {
            if let Some(mgmt) = app.management.as_ref().cloned() {
                app.loading = true;
                app.set_status("Loading entities...");

                let mgmt = mgmt;
                let namespace = app.connection_config
                    .as_ref()
                    .map(|c| c.namespace.clone())
                    .unwrap_or_else(|| "Namespace".to_string());
                let tx = app.bg_tx.clone();

                tokio::spawn(async move {
                    match app::build_tree(mgmt, namespace).await {
                        Ok((tree, flat_nodes)) => {
                            let _ = tx.send(BgEvent::TreeRefreshed { tree, flat_nodes });
                        }
                        Err(e) => {
                            let _ = tx.send(BgEvent::Failed(format!("Refresh failed: {}", e)));
                        }
                    }
                });
            }
            needs_refresh = false;
        }

        // Load detail when selection changes (spawned)
        if app.tree_selected != last_selected && !app.flat_nodes.is_empty() {
            last_selected = app.tree_selected;

            if let Some(mgmt) = app.management.as_ref() {
                if let Some(node) = app.flat_nodes.get(app.tree_selected) {
                    let mgmt = mgmt.clone();
                    let entity_type = node.entity_type.clone();
                    let path = node.path.clone();
                    let tx = app.bg_tx.clone();

                    tokio::spawn(async move {
                        let detail = match entity_type {
                            EntityType::Queue => {
                                match (
                                    mgmt.get_queue(&path).await,
                                    mgmt.get_queue_runtime_info(&path).await,
                                ) {
                                    (Ok(desc), Ok(rt)) => Some(DetailView::Queue(desc, Some(rt))),
                                    (Ok(desc), Err(_)) => Some(DetailView::Queue(desc, None)),
                                    _ => None,
                                }
                            }
                            EntityType::Topic => {
                                match (
                                    mgmt.get_topic(&path).await,
                                    mgmt.get_topic_runtime_info(&path).await,
                                ) {
                                    (Ok(desc), Ok(rt)) => Some(DetailView::Topic(desc, Some(rt))),
                                    (Ok(desc), Err(_)) => Some(DetailView::Topic(desc, None)),
                                    _ => None,
                                }
                            }
                            EntityType::Subscription => {
                                let parts: Vec<&str> = path.split('/').collect();
                                if parts.len() >= 3 {
                                    let topic = parts[0];
                                    let sub = parts[2];
                                    match (
                                        mgmt.get_subscription(topic, sub).await,
                                        mgmt.get_subscription_runtime_info(topic, sub).await,
                                    ) {
                                        (Ok(desc), Ok(rt)) => {
                                            Some(DetailView::Subscription(desc, Some(rt)))
                                        }
                                        (Ok(desc), Err(_)) => {
                                            Some(DetailView::Subscription(desc, None))
                                        }
                                        _ => None,
                                    }
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        };
                        if let Some(d) = detail {
                            let _ = tx.send(BgEvent::DetailLoaded(d));
                        }
                    });
                }
            }
        }

        // Peek messages (spawned)
        if app.status_message == "Peeking messages..." && app.data_plane.is_some() {
            let dp = app.data_plane.clone().unwrap();
            if let Some((path, entity_type)) = app.selected_entity() {
                let is_dlq = app.peek_dlq;
                let is_topic = *entity_type == EntityType::Topic;
                let entity_path = path.to_string();
                app.peek_dlq = false;
                let peek_count = app.pending_peek_count.take()
                    .unwrap_or(app.config.settings.peek_count);
                let tx = app.bg_tx.clone();

                app.set_status("Peeking...");

                if is_topic && is_dlq {
                    let mgmt = app.management.as_ref().cloned();
                    tokio::spawn(async move {
                        let mut all_msgs = Vec::new();
                        if let Some(mgmt) = mgmt {
                            match mgmt.list_subscriptions(&entity_path).await {
                                Ok(subs) => {
                                    for s in &subs {
                                        let dlq_path = format!("{}/subscriptions/{}/$deadletterqueue", entity_path, s.name);
                                        if let Ok(msgs) = dp.peek_messages(&dlq_path, peek_count).await {
                                            all_msgs.extend(msgs);
                                        }
                                    }
                                }
                                Err(e) => {
                                    let _ = tx.send(BgEvent::Failed(format!("Failed to list subscriptions: {}", e)));
                                    return;
                                }
                            }
                        }
                        let _ = tx.send(BgEvent::PeekComplete { messages: all_msgs, is_dlq: true });
                    });
                } else {
                    let peek_path = if is_dlq {
                        format!("{}/$deadletterqueue", entity_path)
                    } else {
                        entity_path
                    };

                    tokio::spawn(async move {
                        match dp.peek_messages(&peek_path, peek_count).await {
                            Ok(msgs) => {
                                let _ = tx.send(BgEvent::PeekComplete { messages: msgs, is_dlq });
                            }
                            Err(e) => {
                                let _ = tx.send(BgEvent::Failed(format!("Peek failed: {}", e)));
                            }
                        }
                    });
                }
            } else {
                app.set_status("Select an entity first");
            }
        }

        // Clear (delete / delete DLQ) — spawn background purge
        let is_clear_delete = app.status_message == "Clearing (delete)..." || app.status_message == "Clearing (delete DLQ)...";
        if is_clear_delete && app.data_plane.is_some() && !app.bg_running {
            let is_dlq = app.status_message == "Clearing (delete DLQ)...";
            if let ActiveModal::ClearOptions { ref entity_path, is_topic, .. } = app.modal {
                let entity_path = entity_path.clone();
                let dp = app.data_plane.clone().unwrap();
                let tx = app.bg_tx.clone();
                let cancel = app.new_cancel_token();
                let mgmt = app.management.as_ref().cloned();

                app.bg_running = true;
                app.modal = ActiveModal::None;
                app.set_status("Preparing purge...");

                tokio::spawn(async move {
                    // Build list of paths to purge
                    let paths: Vec<String> = if is_topic {
                        if let Some(mgmt) = mgmt {
                            match mgmt.list_subscriptions(&entity_path).await {
                                Ok(subs) => subs.iter().map(|s| {
                                    let sub_path = format!("{}/subscriptions/{}", entity_path, s.name);
                                    if is_dlq {
                                        format!("{}/$deadletterqueue", sub_path)
                                    } else {
                                        sub_path
                                    }
                                }).collect(),
                                Err(e) => {
                                    let _ = tx.send(BgEvent::Failed(format!("Failed to list subscriptions: {}", e)));
                                    return;
                                }
                            }
                        } else {
                            return;
                        }
                    } else if is_dlq {
                        vec![format!("{}/$deadletterqueue", entity_path)]
                    } else {
                        vec![entity_path.clone()]
                    };

                    let _ = tx.send(BgEvent::Progress(
                        format!("Purging messages from {} path(s) (Esc to cancel)...", paths.len()),
                    ));

                    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel::<u64>();
                    let tx2 = tx.clone();
                    let progress_task = tokio::spawn(async move {
                        let mut last_reported = 0u64;
                        while let Some(n) = progress_rx.recv().await {
                            if n >= last_reported + 50 {
                                last_reported = n;
                                let _ = tx2.send(BgEvent::Progress(
                                    format!("Deleted {} messages... (Esc to cancel)", n),
                                ));
                            }
                        }
                    });

                    let mut count = 0u64;
                    for path in &paths {
                        match dp.purge_concurrent(path, 32, Some(cancel.clone()), Some(progress_tx.clone())).await {
                            Ok(n) => count += n,
                            Err(e) => {
                                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                                    let _ = tx.send(BgEvent::Cancelled {
                                        message: format!("Cancelled after deleting {} messages", count),
                                    });
                                } else {
                                    let _ = tx.send(BgEvent::Failed(
                                        format!("Purge failed after {} messages: {}", count, e),
                                    ));
                                }
                                drop(progress_tx);
                                let _ = progress_task.await;
                                return;
                            }
                        }
                    }
                    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                        let _ = tx.send(BgEvent::Cancelled {
                            message: format!("Cancelled after deleting {} messages", count),
                        });
                    } else {
                        let _ = tx.send(BgEvent::PurgeComplete { count });
                    }
                    drop(progress_tx);
                    let _ = progress_task.await;
                });
            } else {
                app.set_status("No entity selected");
            }
        }

        // Clear (resend) — spawn background resend of all DLQ messages
        if app.status_message == "Clearing (resend)..." && app.data_plane.is_some() && !app.bg_running {
            if let ActiveModal::ClearOptions { ref base_entity_path, is_topic, .. } = app.modal {
                let base_entity_path = base_entity_path.clone();
                let dp = app.data_plane.clone().unwrap();
                let topic_name = base_entity_path.clone();
                let tx = app.bg_tx.clone();
                let cancel = app.new_cancel_token();
                let mgmt = app.management.as_ref().cloned();

                app.bg_running = true;
                app.modal = ActiveModal::None;
                app.set_status("Preparing DLQ resend...");

                tokio::spawn(async move {
                    // Build (dlq_path, send_path) pairs
                    let pairs: Vec<(String, String)> = if is_topic {
                        if let Some(mgmt) = mgmt {
                            match mgmt.list_subscriptions(&topic_name).await {
                                Ok(subs) => subs.iter().map(|s| {
                                    let sub_path = format!("{}/subscriptions/{}", topic_name, s.name);
                                    let dlq = format!("{}/$deadletterqueue", sub_path);
                                    (dlq, topic_name.clone())
                                }).collect(),
                                Err(e) => {
                                    let _ = tx.send(BgEvent::Failed(format!("Failed to list subscriptions: {}", e)));
                                    return;
                                }
                            }
                        } else {
                            return;
                        }
                    } else {
                        let dlq = format!("{}/$deadletterqueue", &base_entity_path);
                        let target = send_path_owned(&base_entity_path);
                        vec![(dlq, target)]
                    };

                    let _ = tx.send(BgEvent::Progress(
                        format!("Resending all DLQ messages from {} path(s) (Esc to cancel)...", pairs.len()),
                    ));

                    let mut resent = 0u32;
                    let mut errors = 0u32;

                    for (dlq_path, send_target) in &pairs {
                        loop {
                            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                                let _ = tx.send(BgEvent::Cancelled {
                                    message: format!("Cancelled after resending {} messages ({} errors)", resent, errors),
                                });
                                return;
                            }

                            let locked = match dp.peek_lock(dlq_path, 1).await {
                                Ok(Some(msg)) => msg,
                                Ok(None) => break,
                                Err(e) => {
                                    let _ = tx.send(BgEvent::Failed(
                                        format!("Resend failed after {} messages: {}", resent, e),
                                    ));
                                    return;
                                }
                            };

                            let lock_uri = match locked.lock_token_uri {
                                Some(ref uri) => uri.clone(),
                                None => {
                                    errors += 1;
                                    continue;
                                }
                            };

                            let resend_msg = crate::client::models::ServiceBusMessage {
                                body: locked.body.clone(),
                                content_type: locked.broker_properties.content_type.clone(),
                                message_id: locked.broker_properties.message_id.clone(),
                                correlation_id: locked.broker_properties.correlation_id.clone(),
                                session_id: locked.broker_properties.session_id.clone(),
                                label: locked.broker_properties.label.clone(),
                                custom_properties: locked.custom_properties.clone(),
                                ..Default::default()
                            };

                            match dp.send_message(send_target, &resend_msg).await {
                                Ok(_) => {
                                    if dp.complete_message(&lock_uri).await.is_ok() {
                                        resent += 1;
                                    } else {
                                        errors += 1;
                                    }
                                }
                                Err(_) => {
                                    let _ = dp.abandon_message(&lock_uri).await;
                                    errors += 1;
                                }
                            }

                            if (resent + errors) % 50 == 0 {
                                let _ = tx.send(BgEvent::Progress(
                                    format!("Resent {} messages ({} errors)... (Esc to cancel)", resent, errors),
                                ));
                            }
                        }
                    }
                    let _ = tx.send(BgEvent::ResendComplete { resent, errors });
                });
            } else {
                app.set_status("No entity selected");
            }
        }

        // Delete entity (spawned)
        if app.status_message == "Deleting..." {
            if let ActiveModal::ConfirmDelete(ref path) = app.modal {
                let path = path.clone();
                if let Some(mgmt) = app.management.as_ref() {
                    let mgmt = mgmt.clone();
                    let tx = app.bg_tx.clone();
                    app.modal = ActiveModal::None;
                    app.set_status("Deleting entity...");

                    tokio::spawn(async move {
                        let result = if path.contains("/Subscriptions/") {
                            let parts: Vec<&str> = path.split('/').collect();
                            if parts.len() >= 3 {
                                mgmt.delete_subscription(parts[0], parts[2]).await
                            } else {
                                Err(client::ServiceBusError::Operation("Invalid path".into()))
                            }
                        } else {
                            mgmt.delete_queue(&path).await.or(mgmt.delete_topic(&path).await)
                        };

                        match result {
                            Ok(_) => {
                                let _ = tx.send(BgEvent::EntityDeleted {
                                    status: format!("Deleted '{}'", path),
                                });
                            }
                            Err(e) => {
                                let _ = tx.send(BgEvent::Failed(format!("Delete failed: {}", e)));
                            }
                        }
                    });
                } else {
                    app.modal = ActiveModal::None;
                }
            }
        }

        // Submit send message (spawned)
        if app.status_message == "Submitting..." && app.modal == ActiveModal::SendMessage {
            if let Some(dp) = app.data_plane.as_ref() {
                if let Some((path, _)) = app.selected_entity() {
                    let dp = dp.clone();
                    let path = send_path(path).to_string();
                    let msg = app.build_message_from_form();
                    let tx = app.bg_tx.clone();

                    app.set_status("Sending...");

                    tokio::spawn(async move {
                        match dp.send_message(&path, &msg).await {
                            Ok(_) => {
                                let _ = tx.send(BgEvent::SendComplete {
                                    status: "Message sent successfully".to_string(),
                                });
                            }
                            Err(e) => {
                                let _ = tx.send(BgEvent::Failed(format!("Send failed: {}", e)));
                            }
                        }
                    });
                }
            }
        }

        // Submit edit & resend message — modal (spawned)
        if app.status_message == "Submitting..." && app.modal == ActiveModal::EditResend {
            if let Some(dp) = app.data_plane.as_ref() {
                if let Some((path, _)) = app.selected_entity() {
                    let dp = dp.clone();
                    let base_path = send_path(path).to_string();
                    let entity_path = path.to_string();
                    let msg = app.build_message_from_form();
                    let dlq_seq = app.edit_source_dlq_seq.take();
                    let tx = app.bg_tx.clone();

                    app.set_status("Resending...");

                    tokio::spawn(async move {
                        match dp.send_message(&base_path, &msg).await {
                            Ok(_) => {
                                let (status, seq_removed) = if let Some(seq) = dlq_seq {
                                    match dp.remove_from_dlq(&entity_path, seq).await {
                                        Ok(true) => ("Resent and removed from DLQ".to_string(), Some(seq)),
                                        Ok(false) => ("Resent (DLQ message not found to remove)".to_string(), None),
                                        Err(e) => (format!("Resent, but DLQ cleanup failed: {}", e), None),
                                    }
                                } else {
                                    ("Message resent successfully".to_string(), None)
                                };
                                let _ = tx.send(BgEvent::ResendSendComplete {
                                    status,
                                    dlq_seq_removed: seq_removed,
                                    was_inline: false,
                                });
                            }
                            Err(e) => {
                                let _ = tx.send(BgEvent::Failed(format!("Resend failed: {}", e)));
                            }
                        }
                    });
                }
            }
        }

        // Submit inline WYSIWYG edit & resend (spawned)
        if app.status_message == "Submitting..." && app.detail_editing {
            if let Some(dp) = app.data_plane.as_ref() {
                if let Some((path, _)) = app.selected_entity() {
                    let dp = dp.clone();
                    let base_path = send_path(path).to_string();
                    let entity_path = path.to_string();
                    let msg = app.build_message_from_form();
                    let dlq_seq = app.edit_source_dlq_seq.take();
                    let tx = app.bg_tx.clone();

                    app.set_status("Resending...");

                    tokio::spawn(async move {
                        match dp.send_message(&base_path, &msg).await {
                            Ok(_) => {
                                let (status, seq_removed) = if let Some(seq) = dlq_seq {
                                    match dp.remove_from_dlq(&entity_path, seq).await {
                                        Ok(true) => ("Resent and removed from DLQ".to_string(), Some(seq)),
                                        Ok(false) => ("Resent (DLQ message not found to remove)".to_string(), None),
                                        Err(e) => (format!("Resent, but DLQ cleanup failed: {}", e), None),
                                    }
                                } else {
                                    ("Message resent successfully".to_string(), None)
                                };
                                let _ = tx.send(BgEvent::ResendSendComplete {
                                    status,
                                    dlq_seq_removed: seq_removed,
                                    was_inline: true,
                                });
                            }
                            Err(e) => {
                                let _ = tx.send(BgEvent::Failed(format!("Resend failed: {}", e)));
                            }
                        }
                    });
                }
            }
        }

        // Submit create queue (spawned)
        if app.status_message == "Submitting..." && app.modal == ActiveModal::CreateQueue {
            if let Some(mgmt) = app.management.as_ref() {
                let mgmt = mgmt.clone();
                let desc = app.build_queue_from_form();
                let tx = app.bg_tx.clone();
                let name = desc.name.clone();
                app.set_status("Creating queue...");

                tokio::spawn(async move {
                    match mgmt.create_queue(&desc).await {
                        Ok(_) => {
                            let _ = tx.send(BgEvent::EntityCreated {
                                status: format!("Queue '{}' created", name),
                            });
                        }
                        Err(e) => {
                            let _ = tx.send(BgEvent::Failed(format!("Create failed: {}", e)));
                        }
                    }
                });
            }
        }

        // Submit create topic (spawned)
        if app.status_message == "Submitting..." && app.modal == ActiveModal::CreateTopic {
            if let Some(mgmt) = app.management.as_ref() {
                let mgmt = mgmt.clone();
                let desc = app.build_topic_from_form();
                let tx = app.bg_tx.clone();
                let name = desc.name.clone();
                app.set_status("Creating topic...");

                tokio::spawn(async move {
                    match mgmt.create_topic(&desc).await {
                        Ok(_) => {
                            let _ = tx.send(BgEvent::EntityCreated {
                                status: format!("Topic '{}' created", name),
                            });
                        }
                        Err(e) => {
                            let _ = tx.send(BgEvent::Failed(format!("Create failed: {}", e)));
                        }
                    }
                });
            }
        }

        // Submit create subscription (spawned)
        if app.status_message == "Submitting..." && app.modal == ActiveModal::CreateSubscription {
            if let Some(mgmt) = app.management.as_ref() {
                let mgmt = mgmt.clone();
                let desc = app.build_subscription_from_form();
                let tx = app.bg_tx.clone();
                let name = desc.name.clone();
                app.set_status("Creating subscription...");

                tokio::spawn(async move {
                    match mgmt.create_subscription(&desc).await {
                        Ok(_) => {
                            let _ = tx.send(BgEvent::EntityCreated {
                                status: format!("Subscription '{}' created", name),
                            });
                        }
                        Err(e) => {
                            let _ = tx.send(BgEvent::Failed(format!("Create failed: {}", e)));
                        }
                    }
                });
            }
        }

        // Bulk resend from DLQ (messages panel R key)
        if app.status_message == "Bulk resending..." && app.data_plane.is_some() && !app.bg_running {
            if let ActiveModal::ConfirmBulkResend { ref entity_path, count, is_topic } = app.modal {
                let dp = app.data_plane.clone().unwrap();
                let path = entity_path.clone();
                let max_count = count;
                let tx = app.bg_tx.clone();
                let cancel = app.new_cancel_token();
                let mgmt = app.management.as_ref().cloned();

                app.bg_running = true;
                app.modal = ActiveModal::None;
                app.set_status(format!("Resending up to {} messages from DLQ (Esc to cancel)...", max_count));

                tokio::spawn(async move {
                    // Build (dlq_path, send_target) pairs
                    let pairs: Vec<(String, String)> = if is_topic {
                        if let Some(mgmt) = mgmt {
                            match mgmt.list_subscriptions(&path).await {
                                Ok(subs) => subs.iter().map(|s| {
                                    let dlq = format!("{}/subscriptions/{}/$deadletterqueue", path, s.name);
                                    (dlq, path.clone())
                                }).collect(),
                                Err(e) => {
                                    let _ = tx.send(BgEvent::Failed(format!("Failed to list subscriptions: {}", e)));
                                    return;
                                }
                            }
                        } else {
                            return;
                        }
                    } else {
                        let send_target = send_path_owned(&path);
                        let dlq = format!("{}/$deadletterqueue", path);
                        vec![(dlq, send_target)]
                    };

                    let mut resent = 0u32;
                    let mut errors = 0u32;

                    for (dlq_path, send_target) in &pairs {
                        for _ in 0..max_count {
                            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                                let _ = tx.send(BgEvent::Cancelled {
                                    message: format!("Cancelled after resending {} messages ({} errors)", resent, errors),
                                });
                                return;
                            }

                            let locked = match dp.peek_lock(dlq_path, 1).await {
                                Ok(Some(msg)) => msg,
                                Ok(None) => break,
                                Err(e) => {
                                    let _ = tx.send(BgEvent::Failed(format!("Bulk resend failed after {}: {}", resent, e)));
                                    return;
                                }
                            };

                            let lock_uri = match locked.lock_token_uri {
                                Some(ref uri) => uri.clone(),
                                None => { errors += 1; continue; }
                            };

                            let resend_msg = crate::client::models::ServiceBusMessage {
                                body: locked.body.clone(),
                                content_type: locked.broker_properties.content_type.clone(),
                                message_id: locked.broker_properties.message_id.clone(),
                                correlation_id: locked.broker_properties.correlation_id.clone(),
                                session_id: locked.broker_properties.session_id.clone(),
                                label: locked.broker_properties.label.clone(),
                                custom_properties: locked.custom_properties.clone(),
                                ..Default::default()
                            };

                            match dp.send_message(send_target, &resend_msg).await {
                                Ok(_) => {
                                    if dp.complete_message(&lock_uri).await.is_ok() {
                                        resent += 1;
                                    } else {
                                        errors += 1;
                                    }
                                }
                                Err(_) => {
                                    let _ = dp.abandon_message(&lock_uri).await;
                                    errors += 1;
                                }
                            }

                            if (resent + errors) % 50 == 0 {
                                let _ = tx.send(BgEvent::Progress(
                                    format!("Resent {} messages ({} errors)... (Esc to cancel)", resent, errors),
                                ));
                            }
                        }
                    }
                    let _ = tx.send(BgEvent::ResendComplete { resent, errors });
                });
            }
        }

        // Bulk delete messages (messages panel D key)
        if app.status_message == "Bulk deleting..." && app.data_plane.is_some() && !app.bg_running {
            if let ActiveModal::ConfirmBulkDelete { ref entity_path, count: _, is_dlq, is_topic } = app.modal {
                let dp = app.data_plane.clone().unwrap();
                let path = entity_path.clone();
                let was_dlq = is_dlq;
                let tx = app.bg_tx.clone();
                let cancel = app.new_cancel_token();
                let mgmt = app.management.as_ref().cloned();

                app.bg_running = true;
                app.modal = ActiveModal::None;
                app.set_status("Purging messages...");

                tokio::spawn(async move {
                    // Build list of paths to delete from
                    let paths: Vec<String> = if is_topic {
                        if let Some(mgmt) = mgmt {
                            match mgmt.list_subscriptions(&path).await {
                                Ok(subs) => subs.iter().map(|s| {
                                    let sub_path = format!("{}/subscriptions/{}", path, s.name);
                                    if was_dlq {
                                        format!("{}/$deadletterqueue", sub_path)
                                    } else {
                                        sub_path
                                    }
                                }).collect(),
                                Err(e) => {
                                    let _ = tx.send(BgEvent::Failed(format!("Failed to list subscriptions: {}", e)));
                                    return;
                                }
                            }
                        } else {
                            return;
                        }
                    } else {
                        if was_dlq {
                            vec![format!("{}/$deadletterqueue", path)]
                        } else {
                            vec![path.clone()]
                        }
                    };

                    let mut deleted = 0u64;
                    for delete_path in &paths {
                        match dp.purge_concurrent(delete_path, 32, Some(cancel.clone()), None).await {
                            Ok(n) => deleted += n,
                            Err(e) => {
                                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                                    let _ = tx.send(BgEvent::Cancelled {
                                        message: format!("Cancelled after deleting {} messages", deleted),
                                    });
                                } else {
                                    let _ = tx.send(BgEvent::Failed(
                                        format!("Purge failed after {} messages: {}", deleted, e),
                                    ));
                                }
                                return;
                            }
                        }
                    }
                    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                        let _ = tx.send(BgEvent::Cancelled {
                            message: format!("Cancelled after deleting {} messages", deleted),
                        });
                    } else {
                        let _ = tx.send(BgEvent::BulkDeleteComplete { deleted: deleted as u32, was_dlq });
                    }
                });
            }
        }
    }

    Ok(())
}
