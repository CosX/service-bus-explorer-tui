//! Applies results sent back by background tasks to the app state.

use std::time::Instant;

use crate::app::{
    ActiveModal, App, BgEvent, DiscoveryState, FocusPanel, MessageTab, MetricsAvailability,
    MetricsFailure,
};
use crate::client::models::EntityType;
use crate::loaders;

/// Append background-task warnings to a status line.
fn append_warnings(status: &mut String, warnings: &[String]) {
    if !warnings.is_empty() {
        status.push_str(" — ");
        status.push_str(&warnings.join("; "));
    }
}

/// Apply one background event. Returns true when the tree should be reloaded.
pub fn handle(app: &mut App, event: BgEvent) -> bool {
    let mut needs_refresh = false;
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
            app.clear_message_filter();
            needs_refresh = true;
        }
        BgEvent::ResendComplete {
            resent,
            errors,
            warnings,
        } => {
            let mut status = if errors > 0 {
                format!("Resent {} messages ({} errors)", resent, errors)
            } else {
                format!("Resent {} messages", resent)
            };
            append_warnings(&mut status, &warnings);
            if warnings.is_empty() && errors == 0 {
                app.set_status(status);
            } else {
                app.set_error(status);
            }
            app.dlq_messages.clear();
            app.message_selected = 0;
            app.bg_running = false;
            app.clear_message_filter();
            needs_refresh = true;
        }
        BgEvent::BulkDeleteComplete {
            removed,
            was_dlq,
            warnings,
        } => {
            let mut status = format!("Deleted {} messages", removed.len());
            append_warnings(&mut status, &warnings);
            if warnings.is_empty() {
                app.set_status(status);
            } else {
                app.set_error(status);
            }
            // Drop exactly what was removed; anything not found stays visible.
            let removed: std::collections::HashSet<i64> = removed.into_iter().collect();
            let list = if was_dlq {
                &mut app.dlq_messages
            } else {
                &mut app.messages
            };
            list.retain(|m| {
                !m.broker_properties
                    .sequence_number
                    .is_some_and(|s| removed.contains(&s))
            });
            app.message_selected = 0;
            app.selected_message_detail = None;
            app.bg_running = false;
            app.apply_message_filter();
            needs_refresh = true;
        }
        BgEvent::SingleDeleteComplete {
            sequence_number,
            is_dlq,
        } => {
            if is_dlq {
                app.dlq_messages
                    .retain(|m| m.broker_properties.sequence_number != Some(sequence_number));
            } else {
                app.messages
                    .retain(|m| m.broker_properties.sequence_number != Some(sequence_number));
            }
            if app
                .selected_message_detail
                .as_ref()
                .is_some_and(|m| m.broker_properties.sequence_number == Some(sequence_number))
            {
                app.selected_message_detail = None;
            }
            // Rebuild filter indices after message removal
            app.apply_message_filter();
            app.set_status(format!("Deleted message seq #{}", sequence_number));
            app.bg_running = false;
            needs_refresh = true;
        }
        BgEvent::Cancelled { message } => {
            // Dismiss discovery modal if it was in Loading state
            if matches!(
                app.modal,
                ActiveModal::NamespaceDiscovery {
                    state: DiscoveryState::Loading
                }
            ) {
                app.modal = ActiveModal::None;
            }
            app.set_status(message);
            app.bg_running = false;
            needs_refresh = true;
        }
        BgEvent::Failed(msg) => {
            app.set_error(msg);
            app.bg_running = false;
            app.loading = false;
        }
        BgEvent::NamespacesDiscovered { result } => {
            app.bg_running = false;
            app.discovered_namespaces = result.namespaces;
            app.discovery_warnings = result.errors;

            if app.discovered_namespaces.is_empty() {
                let error_msg = if !app.discovery_warnings.is_empty() {
                    app.discovery_warnings.join("; ")
                } else {
                    "No Service Bus namespaces found in your subscriptions".to_string()
                };

                app.modal = ActiveModal::NamespaceDiscovery {
                    state: DiscoveryState::Error(error_msg.clone()),
                };
                app.set_status(format!("Discovery complete: {}", error_msg));
            } else {
                app.modal = ActiveModal::NamespaceDiscovery {
                    state: DiscoveryState::List,
                };
                app.set_status(format!(
                    "Found {} namespace(s). Select one or press 'm' for manual entry.",
                    app.discovered_namespaces.len()
                ));
            }
        }
        BgEvent::DiscoveryFailed(err) => {
            app.bg_running = false;
            app.modal = ActiveModal::NamespaceDiscovery {
                state: DiscoveryState::Error(err.clone()),
            };
            app.set_error(format!("Discovery failed: {}", err));
        }
        BgEvent::TreeRefreshed {
            mut tree,
            flat_nodes,
        } => {
            let q_count = flat_nodes
                .iter()
                .filter(|n| n.entity_type == EntityType::Queue)
                .count();
            let t_count = flat_nodes
                .iter()
                .filter(|n| n.entity_type == EntityType::Topic)
                .count();

            // Preserve expand/collapse state and selection across refreshes
            let prev_selected_id = app.flat_nodes.get(app.tree_selected).map(|n| n.id.clone());

            if let Some(ref old_tree) = app.tree {
                let mut expanded_ids = std::collections::HashSet::new();
                old_tree.collect_expanded_ids(&mut expanded_ids);
                tree.apply_expanded_ids(&expanded_ids);
            }

            app.flat_nodes = tree.flatten();
            app.tree = Some(tree);

            // Restore selection by node ID, fall back to clamping
            if let Some(ref prev_id) = prev_selected_id {
                if let Some(pos) = app.flat_nodes.iter().position(|n| n.id == *prev_id) {
                    app.tree_selected = pos;
                } else if app.tree_selected >= app.flat_nodes.len() {
                    app.tree_selected = app.flat_nodes.len().saturating_sub(1);
                }
            } else if app.tree_selected >= app.flat_nodes.len() {
                app.tree_selected = 0;
            }

            app.loading = false;
            app.last_refresh = Some(Instant::now());
            app.set_status(format!("Loaded {} queues, {} topics", q_count, t_count));

            // Resolve ARM resource ID for Azure Monitor metrics (once per
            // connection; a previous failure is only retried on demand).
            if app.namespace_resource_id.is_none() && !app.metrics.resolution_attempted() {
                loaders::spawn_resource_id_resolve(app, false);
            }
        }
        BgEvent::DetailLoaded { path, view } => {
            // Drop responses for entities the user has already scrolled past.
            let current_path = app.flat_nodes.get(app.tree_selected).map(|n| &n.path);
            if current_path == Some(&path) {
                app.detail_view = *view;
            }
        }
        BgEvent::SubscriptionFilterLoaded {
            topic_name,
            sub_name,
            rule_name,
            sql_expression,
        } => {
            app.bg_running = false;
            app.init_edit_subscription_filter_form(
                &topic_name,
                &sub_name,
                &rule_name,
                &sql_expression,
            );
            app.set_status("Edit the SQL filter and press F2 to update");
        }
        BgEvent::PeekComplete {
            messages,
            is_dlq,
            warnings,
        } => {
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
            app.clear_message_filter();
            let mut status = if is_dlq {
                format!("Peeked {} DLQ messages", count)
            } else {
                format!("Peeked {} messages", count)
            };
            append_warnings(&mut status, &warnings);
            if warnings.is_empty() {
                app.set_status(status);
            } else {
                app.set_error(status);
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
        BgEvent::ResendSendComplete {
            status,
            dlq_seq_removed,
            was_inline,
        } => {
            if let Some(seq) = dlq_seq_removed {
                app.dlq_messages
                    .retain(|m| m.broker_properties.sequence_number != Some(seq));
                app.apply_message_filter();
            }
            app.set_status(status);
            if was_inline {
                app.detail_editing = false;
                app.selected_message_detail = None;
            } else {
                app.modal = ActiveModal::None;
            }
        }
        BgEvent::DestinationEntitiesLoaded { entities } => {
            app.copy_dest_entities = entities;
            app.copy_entity_selected = 0;
            app.copy_entity_list_state.select(Some(0));
            app.bg_running = false;

            if app.copy_dest_entities.is_empty() {
                app.set_status("No entities found in destination namespace");
            } else {
                app.set_status(format!("Loaded {} entities", app.copy_dest_entities.len()));
            }
        }
        BgEvent::MessageCopyComplete { status } => {
            app.set_status(status);
            app.bg_running = false;
            app.copy_source_message = None;
            app.copy_source_entity = None;
            app.copy_dest_entities.clear();
            app.copy_entity_selected = 0;
            app.copy_dest_connection_name = None;
            app.copy_dest_connection_config = None;
            app.copy_destination_entity = None;
        }
        BgEvent::SubscriptionFilterUpdated { status } => {
            app.set_status(status);
            app.modal = ActiveModal::None;
            app.bg_running = false;
        }
        BgEvent::NamespaceResourceIdResolved(id) => {
            let announce = matches!(
                app.metrics,
                MetricsAvailability::Resolving {
                    user_requested: true
                }
            );
            app.namespace_resource_id = Some(id);
            app.metrics = MetricsAvailability::Available;
            app.entity_metrics = None;
            if announce {
                app.set_status(format!(
                    "Metrics available ({})",
                    app.metrics_window.label()
                ));
            }
        }
        BgEvent::NamespaceResourceIdFailed(err) => {
            // Keep the reason: the status bar reported "requires Azure AD"
            // for every failure, including permission and lookup errors.
            let announce = matches!(
                app.metrics,
                MetricsAvailability::Resolving {
                    user_requested: true
                }
            );
            app.metrics = MetricsAvailability::Failed(err);
            if announce {
                let reason = app.metrics.unavailable_reason();
                app.set_error(reason);
            }
        }
        BgEvent::MetricsLoaded(metrics) => {
            app.metrics_pending = false;
            app.metrics_failure = None;
            // Only accept if entity still matches current selection
            let current_entity = app
                .flat_nodes
                .get(app.tree_selected)
                .map(|n| n.label.clone());
            if current_entity.as_deref() == Some(&metrics.entity_name) {
                app.entity_metrics = Some(metrics);
            }
        }
        BgEvent::MetricsFailed {
            entity_name,
            error,
            permanent,
        } => {
            app.metrics_pending = false;
            app.entity_metrics = None;
            // Announce once per failing entity; the backoff stops a retry storm.
            let already_reported = app
                .metrics_failure
                .as_ref()
                .is_some_and(|f| f.entity_name == entity_name);
            if !already_reported {
                app.set_error(format!(
                    "Metrics for '{}' unavailable: {}",
                    entity_name, error
                ));
            }
            app.metrics_failure = Some(MetricsFailure {
                entity_name,
                at: Instant::now(),
                permanent,
            });
        }
    }
    needs_refresh
}
