use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::mpsc::UnboundedSender;

use crate::app::{BgEvent, ClearScope};
use crate::client::{DataPlaneClient, ManagementClient};

pub fn send_path_owned(entity_path: &str) -> String {
    crate::client::entity_path::send_target(entity_path).to_string()
}

/// Data-plane paths a purge should walk for `scope`.
///
/// Topic scopes fan out across their subscriptions; the namespace-wide scopes
/// enumerate every queue, or every subscription of every topic.
pub async fn resolve_purge_paths(
    mgmt: Option<&ManagementClient>,
    scope: &ClearScope,
    is_dlq: bool,
) -> Result<Vec<String>, String> {
    let dlq_suffix = |path: String| {
        if is_dlq {
            format!("{}/$deadletterqueue", path)
        } else {
            path
        }
    };

    match scope {
        ClearScope::Entity(path) => Ok(vec![dlq_suffix(path.clone())]),
        ClearScope::Topic(topic) => Ok(subscription_paths(require(mgmt)?, topic)
            .await?
            .into_iter()
            .map(dlq_suffix)
            .collect()),
        ClearScope::AllQueues => Ok(queue_names(require(mgmt)?)
            .await?
            .into_iter()
            .map(dlq_suffix)
            .collect()),
        ClearScope::AllTopics => {
            let mgmt = require(mgmt)?;
            let mut paths = Vec::new();
            for topic in topic_names(mgmt).await? {
                paths.extend(
                    subscription_paths(mgmt, &topic)
                        .await?
                        .into_iter()
                        .map(&dlq_suffix),
                );
            }
            Ok(paths)
        }
    }
}

/// `(dead-letter path, send target)` pairs a DLQ resend should walk for `scope`.
pub async fn resolve_resend_pairs(
    mgmt: Option<&ManagementClient>,
    scope: &ClearScope,
) -> Result<Vec<(String, String)>, String> {
    let pair =
        |path: &str, target: &str| (format!("{}/$deadletterqueue", path), target.to_string());

    match scope {
        ClearScope::Entity(path) => Ok(vec![pair(path, &send_path_owned(path))]),
        ClearScope::Topic(topic) => Ok(subscription_paths(require(mgmt)?, topic)
            .await?
            .iter()
            .map(|sub| pair(sub, topic))
            .collect()),
        ClearScope::AllQueues => Ok(queue_names(require(mgmt)?)
            .await?
            .iter()
            .map(|q| pair(q, q))
            .collect()),
        ClearScope::AllTopics => {
            let mgmt = require(mgmt)?;
            let mut pairs = Vec::new();
            for topic in topic_names(mgmt).await? {
                for sub in subscription_paths(mgmt, &topic).await? {
                    pairs.push(pair(&sub, &topic));
                }
            }
            Ok(pairs)
        }
    }
}

fn require(mgmt: Option<&ManagementClient>) -> Result<&ManagementClient, String> {
    mgmt.ok_or_else(|| "Not connected".to_string())
}

async fn queue_names(mgmt: &ManagementClient) -> Result<Vec<String>, String> {
    mgmt.list_queues_with_counts()
        .await
        .map_err(|e| format!("Failed to list queues: {}", e))
        .map(|queues| queues.into_iter().map(|(q, _, _)| q.name).collect())
}

async fn topic_names(mgmt: &ManagementClient) -> Result<Vec<String>, String> {
    mgmt.list_topics()
        .await
        .map_err(|e| format!("Failed to list topics: {}", e))
        .map(|topics| topics.into_iter().map(|t| t.name).collect())
}

/// Data-plane paths (lowercase `/subscriptions/`) of every subscription on `topic`.
async fn subscription_paths(mgmt: &ManagementClient, topic: &str) -> Result<Vec<String>, String> {
    mgmt.list_subscriptions(topic)
        .await
        .map_err(|e| format!("Failed to list subscriptions for {}: {}", topic, e))
        .map(|subs| {
            subs.iter()
                .map(|s| format!("{}/subscriptions/{}", topic, s.name))
                .collect()
        })
}

pub async fn resend_dlq_loop(
    dp: &DataPlaneClient,
    pairs: &[(String, String)],
    max_per_path: Option<u32>,
    cancel: &Arc<AtomicBool>,
    tx: &UnboundedSender<BgEvent>,
) -> Result<(u32, u32), String> {
    let mut resent = 0u32;
    let mut errors = 0u32;

    for (dlq_path, send_target) in pairs {
        let mut path_count = 0u32;
        loop {
            if let Some(max) = max_per_path {
                if path_count >= max {
                    break;
                }
            }
            if cancel.load(Ordering::Relaxed) {
                return Err(format!(
                    "Cancelled after resending {} messages ({} errors)",
                    resent, errors
                ));
            }

            let locked = match dp.peek_lock(dlq_path, 1).await {
                Ok(Some(msg)) => msg,
                Ok(None) => break,
                Err(e) => return Err(format!("Resend failed after {} messages: {}", resent, e)),
            };

            let lock_uri = match locked.lock_token_uri {
                Some(ref uri) => uri.clone(),
                None => {
                    errors += 1;
                    path_count += 1;
                    continue;
                }
            };

            match dp.send_message(send_target, &locked.to_sendable()).await {
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

            path_count += 1;
            if (resent + errors).is_multiple_of(50) {
                let _ = tx.send(BgEvent::Progress(format!(
                    "Resent {} messages ({} errors)... (Esc to cancel)",
                    resent, errors
                )));
            }
        }
    }

    Ok((resent, errors))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Entity scopes resolve without a management client (no fan-out needed).
    #[tokio::test]
    async fn entity_purge_paths_target_a_single_path() {
        let scope = ClearScope::Entity("orders".to_string());
        assert_eq!(
            resolve_purge_paths(None, &scope, false).await,
            Ok(vec!["orders".to_string()])
        );
        assert_eq!(
            resolve_purge_paths(None, &scope, true).await,
            Ok(vec!["orders/$deadletterqueue".to_string()])
        );
    }

    #[tokio::test]
    async fn subscription_resend_targets_the_parent_topic() {
        let scope = ClearScope::Entity("events/subscriptions/audit".to_string());
        assert_eq!(
            resolve_resend_pairs(None, &scope).await,
            Ok(vec![(
                "events/subscriptions/audit/$deadletterqueue".to_string(),
                "events".to_string()
            )])
        );
    }

    #[tokio::test]
    async fn queue_resend_targets_itself() {
        let scope = ClearScope::Entity("orders".to_string());
        assert_eq!(
            resolve_resend_pairs(None, &scope).await,
            Ok(vec![(
                "orders/$deadletterqueue".to_string(),
                "orders".to_string()
            )])
        );
    }

    /// Fan-out scopes need a management client to enumerate their children.
    #[tokio::test]
    async fn fan_out_scopes_require_a_connection() {
        for scope in [
            ClearScope::Topic("events".to_string()),
            ClearScope::AllQueues,
            ClearScope::AllTopics,
        ] {
            assert!(resolve_purge_paths(None, &scope, true).await.is_err());
            assert!(resolve_resend_pairs(None, &scope).await.is_err());
        }
    }
}
