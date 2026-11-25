//! Executes [`Command`]s requested by input handling. Every Azure call runs in
//! a spawned task that reports back through `app.bg_tx`.

use std::collections::{BTreeMap, HashSet};
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc::UnboundedSender;

use crate::app::{ActiveModal, App, BgEvent, ClearAction, ClearScope, Command};
use crate::bulk_ops::{
    delete_selected, delivery_guard, resend_dlq_loop, resend_selected, resolve_purge_paths,
    resolve_resend_pairs, send_path_owned, ResendReport,
};
use crate::client::models::{EntityType, ReceivedMessage};
use crate::client::scan::PeekOutcome;
use crate::client::{entity_path, DataPlaneClient};
use crate::loaders;

/// Parallel receive-and-delete workers per purged path.
const PURGE_CONCURRENCY: usize = 32;

/// How often purge progress is reported to the status bar.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

const BUSY: &str = "Another operation is running — wait for it or press Esc to cancel";

type Tx = UnboundedSender<BgEvent>;

fn send_failed(tx: &Tx, message: impl Into<String>) {
    let _ = tx.send(BgEvent::Failed(message.into()));
}

fn send_failed_with<E: std::fmt::Display>(tx: &Tx, context: &str, err: E) {
    send_failed(tx, format!("{}: {}", context, err));
}

/// Run one requested command.
pub fn dispatch(app: &mut App, command: Command) {
    let exclusive = matches!(
        command,
        Command::Clear(_)
            | Command::PurgeAllDlq
            | Command::BulkResend
            | Command::BulkDelete
            | Command::DeleteMessage
            | Command::DiscoverNamespaces
            | Command::LoadSubscriptionFilter
    );
    if exclusive && app.bg_running {
        app.set_error(BUSY);
        return;
    }

    match command {
        Command::Refresh => loaders::spawn_tree_refresh(app),
        Command::DiscoverNamespaces => discover_namespaces(app),
        Command::Peek { count, dlq } => peek(app, count, dlq),
        Command::Clear(pending) => match pending.action {
            ClearAction::DeleteActive => clear_delete(app, pending.scope, false),
            ClearAction::DeleteDlq => clear_delete(app, pending.scope, true),
            ClearAction::ResendDlq => clear_resend(app, pending.scope),
        },
        Command::PurgeAllDlq => purge_all_dlq(app),
        Command::DeleteEntity(path) => delete_entity(app, path),
        Command::SubmitForm => submit_form(app),
        Command::LoadSubscriptionFilter => load_subscription_filter(app),
        Command::LoadDestinationEntities => load_destination_entities(app),
        Command::BulkResend => bulk_resend(app),
        Command::BulkDelete => bulk_delete(app),
        Command::DeleteMessage => delete_message(app),
    }
}

fn require_data_plane(app: &mut App) -> Option<DataPlaneClient> {
    let dp = app.data_plane.clone();
    if dp.is_none() {
        app.set_error("Not connected");
    }
    dp
}

// ──────────────────────────── Namespace discovery ────────────────────────────

fn discover_namespaces(app: &mut App) {
    app.bg_running = true;
    let tx = app.bg_tx.clone();
    let cancel = app.new_cancel_token();

    tokio::spawn(async move {
        let cancelled = || BgEvent::Cancelled {
            message: "Discovery cancelled".into(),
        };
        if cancel.load(Ordering::Relaxed) {
            let _ = tx.send(cancelled());
            return;
        }

        let credential: Arc<dyn azure_core::credentials::TokenCredential> =
            match azure_identity::DeveloperToolsCredential::new(None) {
                Ok(cred) => crate::client::auth::CachedTokenCredential::new(cred),
                Err(e) => {
                    let _ = tx.send(BgEvent::DiscoveryFailed(format!(
                        "Failed to create Azure credential: {}. Try 'az login'",
                        e
                    )));
                    return;
                }
            };

        let client = crate::client::resource_manager::ResourceManagerClient::new(credential);
        let result = client.discover_namespaces(cancel.clone()).await;

        let event = if cancel.load(Ordering::Relaxed) {
            cancelled()
        } else {
            BgEvent::NamespacesDiscovered { result }
        };
        let _ = tx.send(event);
    });
}

// ──────────────────────────── Peek ────────────────────────────

/// Record what a peek did besides reading: messages it dead-lettered and
/// locks it could not release.
fn note_peek(warnings: &mut Vec<String>, source: &str, outcome: &PeekOutcome) {
    if !outcome.dead_lettered_by_peek.is_empty() {
        warnings.push(format!(
            "{}: seq {} had reached MaxDeliveryCount, so peeking it moved it to the DLQ; peek stopped there",
            source,
            outcome
                .dead_lettered_by_peek
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    if outcome.release_failures > 0 {
        warnings.push(format!(
            "{}: {} lock(s) not released; those messages stay hidden until the lock expires",
            source, outcome.release_failures
        ));
    }
}

fn tag_source(messages: Vec<ReceivedMessage>, source: &str) -> Vec<ReceivedMessage> {
    messages
        .into_iter()
        .map(|mut m| {
            m.source_entity = Some(source.to_string());
            m
        })
        .collect()
}

fn peek(app: &mut App, count: usize, dlq: bool) {
    let Some(dp) = require_data_plane(app) else {
        return;
    };
    let Some((path, entity_type)) = app.selected_entity() else {
        app.set_status("Select an entity first");
        return;
    };
    let entity_path = path.to_string();
    let is_topic = *entity_type == EntityType::Topic;
    let mgmt = app.management.clone();
    let tx = app.bg_tx.clone();
    app.set_status("Peeking...");

    tokio::spawn(async move {
        let mut warnings = Vec::new();

        let result: Result<Vec<ReceivedMessage>, String> = if is_topic && dlq {
            // Topics have no DLQ of their own: merge every subscription's.
            match mgmt {
                None => Err("Not connected".to_string()),
                Some(mgmt) => match mgmt.list_subscriptions(&entity_path).await {
                    Err(e) => Err(format!("Failed to list subscriptions: {}", e)),
                    Ok(subs) => {
                        let mut all = Vec::new();
                        for s in &subs {
                            let sub_entity = format!("{}/Subscriptions/{}", entity_path, s.name);
                            let dlq_path = format!("{}/$deadletterqueue", sub_entity);
                            match dp.peek_messages(&dlq_path, count, None).await {
                                Ok(out) => {
                                    note_peek(&mut warnings, &s.name, &out);
                                    all.extend(tag_source(out.messages, &sub_entity));
                                }
                                Err(e) => warnings.push(format!("{}: peek failed: {}", s.name, e)),
                            }
                        }
                        Ok(all)
                    }
                },
            }
        } else if is_topic {
            Err("Topics cannot be peeked directly; peek one of their subscriptions".to_string())
        } else {
            // Dead-letter queues have no delivery limit; active entities do,
            // and peeking past it would dead-letter messages.
            let (peek_path, guard) = if dlq {
                (format!("{}/$deadletterqueue", entity_path), None)
            } else {
                let guard = delivery_guard(mgmt.as_ref(), &entity_path, &mut warnings).await;
                (entity_path.clone(), guard)
            };
            match dp.peek_messages(&peek_path, count, guard).await {
                Ok(out) => {
                    note_peek(&mut warnings, &entity_path, &out);
                    Ok(tag_source(out.messages, &entity_path))
                }
                Err(e) => Err(format!("Peek failed: {}", e)),
            }
        };

        let event = match result {
            Ok(messages) => BgEvent::PeekComplete {
                messages,
                is_dlq: dlq,
                warnings,
            },
            Err(e) => BgEvent::Failed(e),
        };
        let _ = tx.send(event);
    });
}

// ──────────────────────────── Purge / clear ────────────────────────────

/// Purge every path in order, reporting progress, then send the final event.
async fn run_purge(
    dp: DataPlaneClient,
    paths: Vec<String>,
    cancel: Arc<AtomicBool>,
    tx: Tx,
    noun: &'static str,
) {
    let _ = tx.send(BgEvent::Progress(format!(
        "Purging {} from {} path(s) (Esc to cancel)...",
        noun,
        paths.len()
    )));

    // Workers bump a shared counter; a reporter samples it. Nothing queues
    // up no matter how fast messages are deleted.
    let progress = Arc::new(AtomicU64::new(0));
    let finished = Arc::new(AtomicBool::new(false));
    let reporter = {
        let progress = Arc::clone(&progress);
        let finished = Arc::clone(&finished);
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut last = 0;
            while !finished.load(Ordering::Relaxed) {
                tokio::time::sleep(PROGRESS_INTERVAL).await;
                let n = progress.load(Ordering::Relaxed);
                if n != last {
                    last = n;
                    let _ = tx.send(BgEvent::Progress(format!(
                        "Deleted {} {}... (Esc to cancel)",
                        n, noun
                    )));
                }
            }
        })
    };

    let mut count = 0u64;
    let mut failure = None;
    for path in &paths {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        match dp
            .purge_concurrent(
                path,
                PURGE_CONCURRENCY,
                Some(Arc::clone(&cancel)),
                Some(Arc::clone(&progress)),
            )
            .await
        {
            Ok(n) => count += n,
            Err(e) => {
                failure = Some(e);
                break;
            }
        }
    }

    finished.store(true, Ordering::Relaxed);
    let _ = reporter.await;

    let event = if cancel.load(Ordering::Relaxed) {
        BgEvent::Cancelled {
            message: format!("Cancelled after deleting {} {}", count, noun),
        }
    } else if let Some(e) = failure {
        BgEvent::Failed(format!("Purge failed after {} {}: {}", count, noun, e))
    } else {
        BgEvent::PurgeComplete { count }
    };
    let _ = tx.send(event);
}

fn clear_delete(app: &mut App, scope: ClearScope, is_dlq: bool) {
    let Some(dp) = require_data_plane(app) else {
        return;
    };
    let tx = app.bg_tx.clone();
    let cancel = app.new_cancel_token();
    let mgmt = app.management.clone();

    app.bg_running = true;
    app.modal = ActiveModal::None;
    app.set_status("Preparing purge...");

    tokio::spawn(async move {
        match resolve_purge_paths(mgmt.as_ref(), &scope, is_dlq).await {
            Ok(paths) => run_purge(dp, paths, cancel, tx, "messages").await,
            Err(e) => send_failed(&tx, e),
        }
    });
}

fn purge_all_dlq(app: &mut App) {
    let Some(dp) = require_data_plane(app) else {
        return;
    };
    let Some(mgmt) = app.management.clone() else {
        app.set_error("Not connected");
        return;
    };
    let tx = app.bg_tx.clone();
    let cancel = app.new_cancel_token();

    app.bg_running = true;
    app.modal = ActiveModal::None;
    app.set_status("Discovering entities...");

    tokio::spawn(async move {
        let mut paths = Vec::new();
        for scope in [ClearScope::AllQueues, ClearScope::AllTopics] {
            match resolve_purge_paths(Some(&mgmt), &scope, true).await {
                Ok(p) => paths.extend(p),
                Err(e) => {
                    send_failed(&tx, e);
                    return;
                }
            }
        }
        if paths.is_empty() {
            let _ = tx.send(BgEvent::PurgeComplete { count: 0 });
            return;
        }
        run_purge(dp, paths, cancel, tx, "DLQ messages").await;
    });
}

/// Report a finished resend as either a completion or a cancellation.
fn resend_event(report: ResendReport) -> BgEvent {
    if report.cancelled {
        let mut message = report.summary();
        if !report.warnings.is_empty() {
            message.push_str(" — ");
            message.push_str(&report.warnings.join("; "));
        }
        BgEvent::Cancelled { message }
    } else {
        BgEvent::ResendComplete {
            resent: report.resent,
            errors: report.errors,
            warnings: report.warnings,
        }
    }
}

fn clear_resend(app: &mut App, scope: ClearScope) {
    let Some(dp) = require_data_plane(app) else {
        return;
    };
    let tx = app.bg_tx.clone();
    let cancel = app.new_cancel_token();
    let mgmt = app.management.clone();

    app.bg_running = true;
    app.modal = ActiveModal::None;
    app.set_status("Preparing DLQ resend...");

    tokio::spawn(async move {
        let pairs = match resolve_resend_pairs(mgmt.as_ref(), &scope).await {
            Ok(p) => p,
            Err(e) => {
                send_failed(&tx, e);
                return;
            }
        };
        let _ = tx.send(BgEvent::Progress(format!(
            "Resending all DLQ messages from {} path(s) (Esc to cancel)...",
            pairs.len()
        )));

        let progress_tx = tx.clone();
        let progress = move |msg: String| {
            let _ = progress_tx.send(BgEvent::Progress(msg));
        };
        let event = match resend_dlq_loop(&dp, &pairs, None, &cancel, &progress).await {
            Ok(report) => resend_event(report),
            Err(msg) => BgEvent::Failed(msg),
        };
        let _ = tx.send(event);
    });
}

// ──────────────────────────── Entity management ────────────────────────────

fn delete_entity(app: &mut App, path: String) {
    let Some(mgmt) = app.management.clone() else {
        app.modal = ActiveModal::None;
        return;
    };
    // Dispatch on the entity's own type rather than trying each delete.
    let entity_type = app
        .flat_nodes
        .iter()
        .find(|n| n.path == path)
        .map(|n| n.entity_type.clone());
    let tx = app.bg_tx.clone();
    app.modal = ActiveModal::None;
    app.set_status("Deleting entity...");

    tokio::spawn(async move {
        let result = match entity_type {
            Some(EntityType::Subscription) => match entity_path::split_subscription_path(&path) {
                Some((topic, sub)) => mgmt.delete_subscription(topic, sub).await,
                None => {
                    send_failed(&tx, format!("Invalid subscription path '{}'", path));
                    return;
                }
            },
            Some(EntityType::Queue) => mgmt.delete_queue(&path).await,
            Some(EntityType::Topic) => mgmt.delete_topic(&path).await,
            _ => {
                send_failed(&tx, format!("'{}' is not a deletable entity", path));
                return;
            }
        };
        match result {
            Ok(()) => {
                let _ = tx.send(BgEvent::EntityDeleted {
                    status: format!("Deleted '{}'", path),
                });
            }
            Err(e) => send_failed_with(&tx, "Delete failed", e),
        }
    });
}

fn spawn_entity_create<T, Fut>(tx: Tx, kind: &'static str, name: String, op: Fut)
where
    Fut: Future<Output = crate::client::Result<T>> + Send + 'static,
    T: Send + 'static,
{
    tokio::spawn(async move {
        match op.await {
            Ok(_) => {
                let _ = tx.send(BgEvent::EntityCreated {
                    status: format!("{} '{}' created", kind, name),
                });
            }
            Err(e) => send_failed_with(&tx, "Create failed", e),
        }
    });
}

// ──────────────────────────── Form submission ────────────────────────────

/// Submit whichever form is open. The active modal (or inline edit) decides.
fn submit_form(app: &mut App) {
    if app.detail_editing || app.modal == ActiveModal::EditResend {
        return submit_edit_resend(app);
    }
    match app.modal {
        ActiveModal::SendMessage => submit_send(app),
        ActiveModal::CreateQueue => {
            let Some(mgmt) = app.management.clone() else {
                return;
            };
            let desc = app.build_queue_from_form();
            let name = desc.name.clone();
            app.set_status("Creating queue...");
            spawn_entity_create(app.bg_tx.clone(), "Queue", name, async move {
                mgmt.create_queue(&desc).await
            });
        }
        ActiveModal::CreateTopic => {
            let Some(mgmt) = app.management.clone() else {
                return;
            };
            let desc = app.build_topic_from_form();
            let name = desc.name.clone();
            app.set_status("Creating topic...");
            spawn_entity_create(app.bg_tx.clone(), "Topic", name, async move {
                mgmt.create_topic(&desc).await
            });
        }
        ActiveModal::CreateSubscription => {
            let Some(mgmt) = app.management.clone() else {
                return;
            };
            let desc = app.build_subscription_from_form();
            let name = desc.name.clone();
            app.set_status("Creating subscription...");
            spawn_entity_create(app.bg_tx.clone(), "Subscription", name, async move {
                mgmt.create_subscription(&desc).await
            });
        }
        ActiveModal::EditSubscriptionFilter => submit_subscription_filter(app),
        ActiveModal::CopyEditMessage => submit_copy(app),
        _ => {}
    }
}

fn submit_send(app: &mut App) {
    let Some(dp) = require_data_plane(app) else {
        return;
    };
    let Some((path, _)) = app.selected_entity() else {
        return;
    };
    let path = entity_path::send_target(path).to_string();
    let msg = app.build_message_from_form();
    let tx = app.bg_tx.clone();
    app.set_status("Sending...");

    tokio::spawn(async move {
        match dp.send_message(&path, &msg).await {
            Ok(()) => {
                let _ = tx.send(BgEvent::SendComplete {
                    status: "Message sent successfully".to_string(),
                });
            }
            Err(e) => send_failed_with(&tx, "Send failed", e),
        }
    });
}

fn submit_edit_resend(app: &mut App) {
    let was_inline = app.detail_editing;
    let Some(dp) = require_data_plane(app) else {
        return;
    };
    let Some((path, _)) = app.selected_entity() else {
        return;
    };
    let base_path = entity_path::send_target(path).to_string();
    // For topic fan-out the message came from one subscription's DLQ.
    let source_entity = app
        .edit_source_message
        .as_ref()
        .and_then(|m| m.source_entity.clone())
        .unwrap_or_else(|| path.to_string());
    let msg = app.build_message_from_form();
    let dlq_seq = app.edit_source_dlq_seq.take();
    let tx = app.bg_tx.clone();
    app.set_status("Resending...");

    tokio::spawn(async move {
        if let Err(e) = dp.send_message(&base_path, &msg).await {
            send_failed_with(&tx, "Resend failed", e);
            return;
        }
        let (status, removed) = match dlq_seq {
            None => ("Message resent successfully".to_string(), None),
            Some(seq) => match dp.remove_from_dlq(&source_entity, seq).await {
                Ok(true) => ("Resent and removed from DLQ".to_string(), Some(seq)),
                Ok(false) => (
                    "Resent, but the original was not found in the DLQ (it may now exist twice)"
                        .to_string(),
                    None,
                ),
                Err(e) => (format!("Resent, but DLQ cleanup failed: {}", e), None),
            },
        };
        let _ = tx.send(BgEvent::ResendSendComplete {
            status,
            dlq_seq_removed: removed,
            was_inline,
        });
    });
}

fn submit_subscription_filter(app: &mut App) {
    let Some((path, EntityType::Subscription)) = app.selected_entity() else {
        return;
    };
    let Some((topic, sub)) = entity_path::split_subscription_path(path) else {
        app.set_error("Invalid subscription path");
        return;
    };
    let (topic, sub) = (topic.to_string(), sub.to_string());
    let Some(mgmt) = app.management.clone() else {
        return;
    };
    let (rule_name, sql_expression) = app.build_subscription_filter_from_form();
    let tx = app.bg_tx.clone();
    app.bg_running = true;
    app.set_status("Updating subscription filter...");

    tokio::spawn(async move {
        match mgmt
            .upsert_subscription_sql_rule(&topic, &sub, &rule_name, &sql_expression)
            .await
        {
            Ok(()) => {
                let _ = tx.send(BgEvent::SubscriptionFilterUpdated {
                    status: format!("Updated '{}' filter for subscription '{}'", rule_name, sub),
                });
            }
            Err(e) => send_failed_with(&tx, "Failed to update subscription filter", e),
        }
    });
}

fn submit_copy(app: &mut App) {
    let (Some(dest_entity), Some(conn_cfg), Some(conn_name)) = (
        app.copy_destination_entity.clone(),
        app.copy_dest_connection_config.clone(),
        app.copy_dest_connection_name.clone(),
    ) else {
        app.set_error("Missing destination configuration");
        app.modal = ActiveModal::None;
        return;
    };
    let msg = app.build_message_from_form();
    let tx = app.bg_tx.clone();
    app.bg_running = true;
    app.modal = ActiveModal::None;
    app.set_status("Copying...");

    tokio::spawn(async move {
        let dest_dp = DataPlaneClient::new(conn_cfg);
        match dest_dp.send_message(&dest_entity, &msg).await {
            Ok(()) => {
                let _ = tx.send(BgEvent::MessageCopyComplete {
                    status: format!(
                        "Message copied to '{}' in connection '{}'",
                        dest_entity, conn_name
                    ),
                });
            }
            Err(e) => send_failed_with(&tx, "Copy failed", e),
        }
    });
}

// ──────────────────────────── Loading for forms ────────────────────────────

fn load_subscription_filter(app: &mut App) {
    let Some((path, EntityType::Subscription)) = app.selected_entity() else {
        return;
    };
    let Some((topic, sub)) = entity_path::split_subscription_path(path) else {
        app.set_error("Invalid subscription path");
        return;
    };
    let (topic_name, sub_name) = (topic.to_string(), sub.to_string());
    let Some(mgmt) = app.management.clone() else {
        return;
    };
    let tx = app.bg_tx.clone();
    app.bg_running = true;
    app.set_status("Loading subscription filters...");

    tokio::spawn(async move {
        let rules = match mgmt.list_subscription_rules(&topic_name, &sub_name).await {
            Ok(rules) => rules,
            Err(e) => {
                send_failed_with(&tx, "Failed to load subscription filters", e);
                return;
            }
        };
        let selected = rules
            .iter()
            .find(|r| r.name == "$Default")
            .or_else(|| rules.first());

        let (rule_name, sql_expression) = match selected {
            // Correlation filters have no SQL form; saving one here would
            // replace it with a SQL filter and change what the subscription
            // receives.
            Some(r) if !r.filter_kind.is_sql_editable() => {
                send_failed(
                    &tx,
                    format!(
                        "Rule '{}' is a {:?} filter, which cannot be edited as SQL here",
                        r.name, r.filter_kind
                    ),
                );
                return;
            }
            Some(r) => (r.name.clone(), r.sql_expression.clone()),
            None => ("$Default".to_string(), "1=1".to_string()),
        };

        let _ = tx.send(BgEvent::SubscriptionFilterLoaded {
            topic_name,
            sub_name,
            rule_name,
            sql_expression,
        });
    });
}

fn load_destination_entities(app: &mut App) {
    if app.modal != ActiveModal::CopySelectEntity {
        return;
    }
    let Some(conn_cfg) = app.copy_dest_connection_config.clone() else {
        return;
    };
    let tx = app.bg_tx.clone();
    app.bg_running = true;

    tokio::spawn(async move {
        match App::fetch_destination_entities(conn_cfg).await {
            Ok(entities) => {
                let _ = tx.send(BgEvent::DestinationEntitiesLoaded { entities });
            }
            Err(e) => send_failed_with(&tx, "Failed to load entities", e),
        }
    });
}

// ──────────────────────────── Peeked-message operations ────────────────────────────

/// Group peeked messages by the entity they came from, collecting sequence
/// numbers. Messages without one cannot be targeted and are counted.
fn group_by_source(
    messages: &[ReceivedMessage],
    default_source: &str,
) -> (BTreeMap<String, HashSet<i64>>, usize) {
    let mut groups: BTreeMap<String, HashSet<i64>> = BTreeMap::new();
    let mut untargetable = 0;
    for m in messages {
        match m.broker_properties.sequence_number {
            Some(seq) => {
                let source = m.source_entity.as_deref().unwrap_or(default_source);
                groups.entry(source.to_string()).or_default().insert(seq);
            }
            None => untargetable += 1,
        }
    }
    (groups, untargetable)
}

fn bulk_resend(app: &mut App) {
    let ActiveModal::ConfirmBulkResend {
        ref entity_path, ..
    } = app.modal
    else {
        return;
    };
    let entity_path = entity_path.clone();
    let Some(dp) = require_data_plane(app) else {
        return;
    };
    let (groups, untargetable) = group_by_source(&app.dlq_messages, &entity_path);
    let groups: Vec<(String, String, HashSet<i64>)> = groups
        .into_iter()
        .map(|(source, seqs)| {
            (
                format!("{}/$deadletterqueue", source),
                send_path_owned(&source),
                seqs,
            )
        })
        .collect();
    let total: usize = groups.iter().map(|(_, _, s)| s.len()).sum();
    let tx = app.bg_tx.clone();
    let cancel = app.new_cancel_token();

    app.bg_running = true;
    app.modal = ActiveModal::None;
    app.set_status(format!(
        "Resending {} peeked DLQ messages (Esc to cancel)...",
        total
    ));

    tokio::spawn(async move {
        let event = match resend_selected(&dp, &groups, &cancel).await {
            Ok(mut report) => {
                if untargetable > 0 {
                    report.warnings.push(format!(
                        "{} message(s) had no sequence number and were skipped",
                        untargetable
                    ));
                }
                resend_event(report)
            }
            Err(e) => BgEvent::Failed(e),
        };
        let _ = tx.send(event);
    });
}

fn bulk_delete(app: &mut App) {
    let ActiveModal::ConfirmBulkDelete {
        ref entity_path,
        is_dlq,
        ..
    } = app.modal
    else {
        return;
    };
    let entity_path = entity_path.clone();
    let Some(dp) = require_data_plane(app) else {
        return;
    };
    let messages = if is_dlq {
        &app.dlq_messages
    } else {
        &app.messages
    };
    let (groups, untargetable) = group_by_source(messages, &entity_path);
    let mgmt = app.management.clone();
    let tx = app.bg_tx.clone();
    let cancel = app.new_cancel_token();

    app.bg_running = true;
    app.modal = ActiveModal::None;
    app.set_status("Deleting peeked messages (Esc to cancel)...");

    tokio::spawn(async move {
        let mut warnings = Vec::new();
        if untargetable > 0 {
            warnings.push(format!(
                "{} message(s) had no sequence number and were skipped",
                untargetable
            ));
        }
        let mut targets = Vec::with_capacity(groups.len());
        for (source, seqs) in groups {
            if is_dlq {
                targets.push((format!("{}/$deadletterqueue", source), None, seqs));
            } else {
                let guard = delivery_guard(mgmt.as_ref(), &source, &mut warnings).await;
                targets.push((source, guard, seqs));
            }
        }

        let event = match delete_selected(&dp, &targets, &cancel).await {
            Ok((removed, problems, cancelled)) => {
                warnings.extend(problems);
                if cancelled {
                    BgEvent::Cancelled {
                        message: format!("Cancelled after deleting {} messages", removed.len()),
                    }
                } else {
                    BgEvent::BulkDeleteComplete {
                        removed,
                        was_dlq: is_dlq,
                        warnings,
                    }
                }
            }
            Err(e) => BgEvent::Failed(e),
        };
        let _ = tx.send(event);
    });
}

fn delete_message(app: &mut App) {
    let ActiveModal::ConfirmSingleDelete {
        ref entity_path,
        sequence_number,
        is_dlq,
    } = app.modal
    else {
        return;
    };
    let entity_path = entity_path.clone();
    let Some(dp) = require_data_plane(app) else {
        return;
    };
    let mgmt = app.management.clone();
    let tx = app.bg_tx.clone();

    app.bg_running = true;
    app.modal = ActiveModal::None;
    app.set_status("Deleting message...");

    tokio::spawn(async move {
        let mut warnings = Vec::new();
        let (path, guard) = if is_dlq {
            (format!("{}/$deadletterqueue", entity_path), None)
        } else {
            let guard = delivery_guard(mgmt.as_ref(), &entity_path, &mut warnings).await;
            (entity_path.clone(), guard)
        };
        let targets: HashSet<i64> = [sequence_number].into_iter().collect();

        let event = match dp.remove_by_sequence(&path, &targets, guard, None).await {
            Ok(report) if report.completed.contains(&sequence_number) => {
                BgEvent::SingleDeleteComplete {
                    sequence_number,
                    is_dlq,
                }
            }
            Ok(report) => {
                warnings.extend(report.problems());
                BgEvent::Failed(format!(
                    "Message seq #{} not deleted: {}",
                    sequence_number,
                    warnings.join("; ")
                ))
            }
            Err(e) => BgEvent::Failed(format!("Delete failed: {}", e)),
        };
        let _ = tx.send(event);
    });
}

#[cfg(test)]
mod tests {
    use super::group_by_source;
    use crate::client::scan::fake::message;

    #[test]
    fn peeked_messages_group_by_their_source_subscription() {
        let mut a = message(1, 1);
        a.source_entity = Some("t/Subscriptions/a".to_string());
        let mut b = message(2, 1);
        b.source_entity = Some("t/Subscriptions/b".to_string());
        let c = message(3, 1); // no source: falls back to the selected entity
        let (groups, untargetable) = group_by_source(&[a, b, c], "t");
        assert_eq!(untargetable, 0);
        assert_eq!(groups.len(), 3);
        assert!(groups["t/Subscriptions/a"].contains(&1));
        assert!(groups["t"].contains(&3));
    }
}
