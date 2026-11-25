//! Background loads the main loop schedules on its own: the entity tree, the
//! detail pane, Azure Monitor metrics and the namespace's ARM resource ID.

use std::time::Duration;

use crate::app::{App, BgEvent, DetailView, MetricsAvailability};
use crate::client::models::EntityType;
use crate::client::resource_manager::ResourceManagerClient;
use crate::client::{entity_path, ManagementClient};

/// Overall budget for resolving the ARM resource ID, token acquisition included.
const RESOURCE_ID_RESOLVE_TIMEOUT: Duration = Duration::from_secs(60);

/// Reload the entity tree.
pub fn spawn_tree_refresh(app: &mut App) {
    let Some(mgmt) = app.management.clone() else {
        return;
    };
    app.loading = true;
    app.set_status("Loading entities...");

    let namespace = app
        .connection_config
        .as_ref()
        .map(|c| c.namespace.clone())
        .unwrap_or_else(|| "Namespace".to_string());
    let tx = app.bg_tx.clone();

    tokio::spawn(async move {
        let event = match crate::app::build_tree(mgmt, namespace).await {
            Ok((tree, flat_nodes)) => BgEvent::TreeRefreshed { tree, flat_nodes },
            Err(e) => BgEvent::Failed(format!("Refresh failed: {}", e)),
        };
        let _ = tx.send(event);
    });
}

/// Load the detail pane (and metrics) for the selected tree node.
pub fn spawn_detail_load(app: &mut App) {
    let Some(mgmt) = app.management.clone() else {
        return;
    };
    let Some(node) = app.flat_nodes.get(app.tree_selected) else {
        return;
    };
    let entity_type = node.entity_type.clone();
    let path = node.path.clone();
    let label = node.label.clone();
    let tx = app.bg_tx.clone();

    let load_type = entity_type.clone();
    tokio::spawn(async move {
        if let Some(view) = load_detail(&mgmt, &load_type, &path).await {
            let _ = tx.send(BgEvent::DetailLoaded {
                path,
                view: Box::new(view),
            });
        }
    });

    let wants_metrics = matches!(entity_type, EntityType::Queue | EntityType::Topic);
    if wants_metrics && metrics_wanted_for(app, &label) {
        spawn_metrics_fetch(app, &label);
    } else {
        app.entity_metrics = None;
    }
}

async fn load_detail(
    mgmt: &ManagementClient,
    entity_type: &EntityType,
    path: &str,
) -> Option<DetailView> {
    match entity_type {
        EntityType::Queue => {
            let (desc, rt) = tokio::join!(mgmt.get_queue(path), mgmt.get_queue_runtime_info(path));
            Some(DetailView::Queue(desc.ok()?, rt.ok()))
        }
        EntityType::Topic => {
            let (desc, rt) = tokio::join!(mgmt.get_topic(path), mgmt.get_topic_runtime_info(path));
            let desc = desc.ok()?;
            let rt = match rt {
                Ok(mut rt) => {
                    // A topic's counts are the sum of its subscriptions'.
                    if let Ok(subs) = mgmt.list_subscriptions_with_counts(path).await {
                        let (active, dlq) = subs
                            .iter()
                            .fold((0, 0), |(a, d), (_, sa, sd)| (a + sa, d + sd));
                        rt.active_message_count = active;
                        rt.dead_letter_message_count = dlq;
                    }
                    Some(rt)
                }
                Err(_) => None,
            };
            Some(DetailView::Topic(desc, rt))
        }
        EntityType::Subscription => {
            let (topic, sub) = entity_path::split_subscription_path(path)?;
            let (desc, rt, rules) = tokio::join!(
                mgmt.get_subscription(topic, sub),
                mgmt.get_subscription_runtime_info(topic, sub),
                mgmt.list_subscription_rules(topic, sub),
            );
            Some(DetailView::Subscription(
                desc.ok()?,
                rt.ok(),
                rules.unwrap_or_default(),
            ))
        }
        _ => None,
    }
}

/// Whether metrics should be fetched for `entity_name` right now.
fn metrics_wanted_for(app: &App, entity_name: &str) -> bool {
    app.metrics.is_available()
        && app.metrics_enabled
        && !app.metrics_pending
        && !app
            .metrics_failure
            .as_ref()
            .is_some_and(|f| f.blocks(entity_name))
}

/// Re-fetch metrics after they were cleared (window cycled, toggled back on,
/// or a failure's backoff expired). `selection_settled` keeps this from firing
/// while the user is still scrolling.
pub fn refetch_metrics_if_needed(app: &mut App, selection_settled: bool) {
    if !selection_settled || app.entity_metrics.is_some() {
        return;
    }
    let target = app
        .flat_nodes
        .get(app.tree_selected)
        .filter(|n| matches!(n.entity_type, EntityType::Queue | EntityType::Topic))
        .map(|n| n.label.clone());
    if let Some(name) = target {
        if metrics_wanted_for(app, &name) {
            spawn_metrics_fetch(app, &name);
        }
    }
}

/// Spawn an Azure Monitor metrics fetch for the given entity.
fn spawn_metrics_fetch(app: &mut App, entity_name: &str) {
    let (Some(resource_id), Some(credential)) = (
        app.namespace_resource_id.clone(),
        app.connection_config
            .as_ref()
            .and_then(|cfg| cfg.azure_ad_credential()),
    ) else {
        return;
    };

    app.metrics_pending = true;
    let rm = ResourceManagerClient::new(credential);
    let entity_name = entity_name.to_string();
    let timespan = app.metrics_window.timespan().to_string();
    let interval = app.metrics_window.interval().to_string();
    let tx = app.bg_tx.clone();

    tokio::spawn(async move {
        let event = match rm
            .query_entity_metrics(&resource_id, &entity_name, &timespan, &interval)
            .await
        {
            Ok(metrics) => BgEvent::MetricsLoaded(metrics),
            Err(e) => BgEvent::MetricsFailed {
                entity_name,
                permanent: e.is_permanent(),
                error: e.to_string(),
            },
        };
        let _ = tx.send(event);
    });
}

/// Resolve the namespace's ARM resource ID, which Azure Monitor metrics need.
///
/// Only Azure AD connections can reach ARM; SAS connections stay `Unsupported`.
pub fn spawn_resource_id_resolve(app: &mut App, user_requested: bool) {
    let Some((credential, fqdn)) = app.connection_config.as_ref().and_then(|cfg| {
        cfg.azure_ad_credential()
            .map(|cred| (cred, cfg.namespace.clone()))
    }) else {
        app.metrics = MetricsAvailability::Unsupported;
        return;
    };

    let tx = app.bg_tx.clone();
    let rm = ResourceManagerClient::new(credential);

    app.metrics = MetricsAvailability::Resolving { user_requested };
    tokio::spawn(async move {
        // The HTTP client has its own per-request timeout; this also bounds
        // token acquisition, which can otherwise block indefinitely.
        let resolved = tokio::time::timeout(
            RESOURCE_ID_RESOLVE_TIMEOUT,
            rm.resolve_namespace_resource_id(&fqdn),
        )
        .await;

        let event = match resolved {
            Ok(Ok(id)) => BgEvent::NamespaceResourceIdResolved(id),
            Ok(Err(e)) => BgEvent::NamespaceResourceIdFailed(e.to_string()),
            Err(_) => BgEvent::NamespaceResourceIdFailed(format!(
                "timed out after {}s resolving '{}'",
                RESOURCE_ID_RESOLVE_TIMEOUT.as_secs(),
                fqdn
            )),
        };
        let _ = tx.send(event);
    });
}
