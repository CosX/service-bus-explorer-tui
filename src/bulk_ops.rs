use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::app::ClearScope;
use crate::client::scan::{
    scan_for_sequences, LockingReceiver, MessageSender, ScanLimits, DEFAULT_MAX_SCAN,
};
use crate::client::ManagementClient;

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

/// Azure's default `MaxDeliveryCount`, assumed when the real one cannot be read.
pub const DEFAULT_MAX_DELIVERY_COUNT: i32 = 10;

/// Give up on a DLQ path after this many sends in a row fail: the target is
/// almost certainly unusable (disabled, missing Send claim, sessions, ...).
pub const MAX_CONSECUTIVE_SEND_FAILURES: u32 = 5;

/// The `MaxDeliveryCount` of the queue or subscription at `entity_path`
/// (management or data-plane casing).
pub async fn max_delivery_count(
    mgmt: &ManagementClient,
    entity_path: &str,
) -> Result<Option<i32>, String> {
    let base = entity_path.trim_end_matches("/$deadletterqueue");
    if let Some((topic, sub)) = crate::client::entity_path::split_subscription_path(base) {
        mgmt.get_subscription(topic, sub)
            .await
            .map(|d| d.max_delivery_count)
            .map_err(|e| e.to_string())
    } else {
        mgmt.get_queue(base)
            .await
            .map(|d| d.max_delivery_count)
            .map_err(|e| e.to_string())
    }
}

/// Delivery limit to guard a lock-based operation on an active entity.
///
/// Falls back to Azure's default when the entity cannot be read, so the guard
/// is never silently disabled; the fallback is reported through `warnings`.
pub async fn delivery_guard(
    mgmt: Option<&ManagementClient>,
    entity_path: &str,
    warnings: &mut Vec<String>,
) -> Option<i32> {
    let looked_up = match mgmt {
        Some(m) => max_delivery_count(m, entity_path).await,
        None => Err("not connected".to_string()),
    };
    match looked_up {
        Ok(Some(n)) => Some(n),
        Ok(None) => Some(DEFAULT_MAX_DELIVERY_COUNT),
        Err(e) => {
            warnings.push(format!(
                "could not read MaxDeliveryCount for {} ({}); assuming {}",
                entity_path, e, DEFAULT_MAX_DELIVERY_COUNT
            ));
            Some(DEFAULT_MAX_DELIVERY_COUNT)
        }
    }
}

/// Outcome of a DLQ resend.
#[derive(Debug, Default)]
pub struct ResendReport {
    pub resent: u32,
    pub errors: u32,
    pub warnings: Vec<String>,
    pub cancelled: bool,
}

impl ResendReport {
    pub fn summary(&self) -> String {
        let mut s = format!("Resent {} messages", self.resent);
        if self.errors > 0 {
            s.push_str(&format!(" ({} errors)", self.errors));
        }
        if self.cancelled {
            s.push_str(" — cancelled");
        }
        s
    }
}

async fn release_held<C: LockingReceiver + ?Sized>(
    client: &C,
    held: &[String],
    path: &str,
    report: &mut ResendReport,
) {
    let mut failures = 0;
    for uri in held {
        if client.release(uri).await.is_err() {
            failures += 1;
        }
    }
    if failures > 0 {
        report.warnings.push(format!(
            "{}: {} lock(s) not released; those messages reappear when the lock expires",
            path, failures
        ));
    }
}

/// Resend every message in each `(dead-letter path, send target)` pair.
///
/// Terminates on every input: a message whose send fails keeps its lock until
/// the path is finished (so the loop moves on instead of re-reading it), the
/// path is abandoned after [`MAX_CONSECUTIVE_SEND_FAILURES`], and meeting an
/// already-seen sequence number (an expired lock) ends the path rather than
/// resending it a second time.
pub async fn resend_dlq_loop<C>(
    client: &C,
    pairs: &[(String, String)],
    max_per_path: Option<u32>,
    cancel: &AtomicBool,
    progress: &(dyn Fn(String) + Send + Sync),
) -> Result<ResendReport, String>
where
    C: LockingReceiver + MessageSender + ?Sized,
{
    let mut report = ResendReport::default();

    for (dlq_path, send_target) in pairs {
        let mut path_count = 0u32;
        let mut consecutive_failures = 0u32;
        let mut seen: HashSet<i64> = HashSet::new();
        let mut held: Vec<String> = Vec::new();
        let mut duplicated = 0u32;

        loop {
            if max_per_path.is_some_and(|max| path_count >= max) {
                break;
            }
            if cancel.load(Ordering::Relaxed) {
                report.cancelled = true;
                release_held(client, &held, dlq_path, &mut report).await;
                return Ok(report);
            }

            let locked = match client.lock_next(dlq_path).await {
                Ok(Some(msg)) => msg,
                Ok(None) => break,
                Err(e) => {
                    release_held(client, &held, dlq_path, &mut report).await;
                    return Err(format!(
                        "Resend failed after {} messages: {}",
                        report.resent, e
                    ));
                }
            };
            path_count += 1;

            let Some(lock_uri) = locked.lock_token_uri.clone() else {
                report.errors += 1;
                continue;
            };

            if let Some(seq) = locked.broker_properties.sequence_number {
                if !seen.insert(seq) {
                    held.push(lock_uri);
                    report.warnings.push(format!(
                        "{}: stopped on reaching messages it had already tried",
                        dlq_path
                    ));
                    break;
                }
            }

            match client.send(send_target, &locked.to_sendable()).await {
                Ok(()) => {
                    consecutive_failures = 0;
                    match client.complete(&lock_uri).await {
                        Ok(()) => report.resent += 1,
                        Err(_) => {
                            report.errors += 1;
                            duplicated += 1;
                        }
                    }
                }
                Err(e) => {
                    report.errors += 1;
                    consecutive_failures += 1;
                    held.push(lock_uri);
                    if consecutive_failures >= MAX_CONSECUTIVE_SEND_FAILURES {
                        report.warnings.push(format!(
                            "{} -> {}: gave up after {} consecutive send failures: {}",
                            dlq_path, send_target, consecutive_failures, e
                        ));
                        break;
                    }
                }
            }

            if (report.resent + report.errors).is_multiple_of(50) {
                progress(format!(
                    "Resent {} messages ({} errors)... (Esc to cancel)",
                    report.resent, report.errors
                ));
            }
        }

        release_held(client, &held, dlq_path, &mut report).await;
        if duplicated > 0 {
            report.warnings.push(format!(
                "{}: {} message(s) were resent but could not be removed from the DLQ (now duplicated)",
                dlq_path, duplicated
            ));
        }
    }

    Ok(report)
}

/// Resend specific DLQ messages, grouped by
/// `(dead-letter path, send target, sequence numbers)`.
///
/// Each group is one forward scan of its DLQ: matching messages are sent from
/// the freshly locked copy (exact bytes) and completed; nothing is resent
/// unless its original is in hand.
pub async fn resend_selected<C>(
    client: &C,
    groups: &[(String, String, HashSet<i64>)],
    cancel: &AtomicBool,
) -> Result<ResendReport, String>
where
    C: LockingReceiver + MessageSender + ?Sized,
{
    let mut report = ResendReport::default();
    let limits = ScanLimits {
        max_scan: DEFAULT_MAX_SCAN,
        max_delivery_count: None,
    };

    for (dlq_path, target, seqs) in groups {
        let target = target.as_str();
        let scan = scan_for_sequences(client, dlq_path, seqs, limits, Some(cancel), |msg| {
            let outgoing = msg.to_sendable();
            async move {
                client
                    .send(target, &outgoing)
                    .await
                    .map_err(|e| e.to_string())
            }
        })
        .await
        .map_err(|e| format!("Resend failed after {} messages: {}", report.resent, e))?;

        report.resent += scan.completed.len() as u32;
        report.errors +=
            (scan.failed.len() + scan.complete_failed.len() + scan.not_found.len()) as u32;
        for problem in scan.problems() {
            report.warnings.push(format!("{}: {}", dlq_path, problem));
        }
        if scan.cancelled {
            report.cancelled = true;
            break;
        }
    }

    Ok(report)
}

/// Delete specific messages, grouped by `(path, delivery limit, sequence numbers)`.
///
/// Returns the sequence numbers actually removed and any problems.
pub async fn delete_selected<C>(
    client: &C,
    groups: &[(String, Option<i32>, HashSet<i64>)],
    cancel: &AtomicBool,
) -> Result<(Vec<i64>, Vec<String>, bool), String>
where
    C: LockingReceiver + ?Sized,
{
    let mut removed = Vec::new();
    let mut warnings = Vec::new();

    for (path, max_delivery_count, seqs) in groups {
        let limits = ScanLimits {
            max_scan: DEFAULT_MAX_SCAN,
            max_delivery_count: *max_delivery_count,
        };
        let scan = scan_for_sequences(client, path, seqs, limits, Some(cancel), |_| async {
            Ok(())
        })
        .await
        .map_err(|e| format!("Delete failed after {} messages: {}", removed.len(), e))?;

        removed.extend(scan.completed.iter().copied());
        for problem in scan.problems() {
            warnings.push(format!("{}: {}", path, problem));
        }
        if scan.cancelled {
            return Ok((removed, warnings, true));
        }
    }

    Ok((removed, warnings, false))
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

    // ── resend loop termination ──

    use crate::client::scan::fake::{message, FakeEntity};

    fn no_progress() -> impl Fn(String) + Send + Sync {
        |_| {}
    }

    fn pair() -> Vec<(String, String)> {
        vec![("q/$deadletterqueue".to_string(), "q".to_string())]
    }

    #[tokio::test]
    async fn resend_moves_every_message_and_empties_the_dlq() {
        let fake = FakeEntity::with((1..=4).map(|s| message(s, 1)).collect(), None);
        let cancel = AtomicBool::new(false);
        let report = resend_dlq_loop(&fake, &pair(), None, &cancel, &no_progress())
            .await
            .expect("resend");
        assert_eq!(report.resent, 4);
        assert_eq!(report.errors, 0);
        fake.snapshot(|st| {
            assert!(st.available.is_empty());
            assert_eq!(st.sent.len(), 4);
        });
    }

    #[tokio::test]
    async fn persistent_send_failure_terminates_instead_of_spinning() {
        let fake = FakeEntity::with((1..=20).map(|s| message(s, 1)).collect(), None);
        fake.state.lock().expect("lock").send_fails = true;
        let cancel = AtomicBool::new(false);
        let report = resend_dlq_loop(&fake, &pair(), None, &cancel, &no_progress())
            .await
            .expect("resend");
        assert_eq!(report.resent, 0);
        assert_eq!(report.errors, MAX_CONSECUTIVE_SEND_FAILURES);
        assert!(report.warnings.iter().any(|w| w.contains("gave up")));
        fake.snapshot(|st| {
            assert!(st.locked.is_empty(), "held locks released");
            assert_eq!(st.available.len(), 20, "nothing lost");
        });
    }

    #[tokio::test]
    async fn one_poison_message_does_not_block_the_rest() {
        let fake = FakeEntity::with((1..=5).map(|s| message(s, 1)).collect(), None);
        fake.state.lock().expect("lock").send_fails_for = vec!["m1".to_string()];
        let cancel = AtomicBool::new(false);
        let report = resend_dlq_loop(&fake, &pair(), None, &cancel, &no_progress())
            .await
            .expect("resend");
        assert_eq!(report.resent, 4);
        assert_eq!(report.errors, 1);
        fake.snapshot(|st| {
            let left: Vec<i64> = st
                .available
                .iter()
                .filter_map(|m| m.broker_properties.sequence_number)
                .collect();
            assert_eq!(left, vec![1], "only the poison message remains");
        });
    }

    #[tokio::test]
    async fn resend_selected_only_touches_the_chosen_messages() {
        let fake = FakeEntity::with((1..=6).map(|s| message(s, 1)).collect(), None);
        let cancel = AtomicBool::new(false);
        let groups = vec![(
            "q/$deadletterqueue".to_string(),
            "q".to_string(),
            [2, 4].into_iter().collect::<HashSet<i64>>(),
        )];
        let report = resend_selected(&fake, &groups, &cancel)
            .await
            .expect("resend");
        assert_eq!(report.resent, 2);
        fake.snapshot(|st| {
            let sent: Vec<String> = st
                .sent
                .iter()
                .filter_map(|(_, m)| m.message_id.clone())
                .collect();
            assert_eq!(sent, vec!["m2".to_string(), "m4".to_string()]);
            assert_eq!(st.available.len(), 4);
        });
    }

    #[tokio::test]
    async fn resend_preserves_binary_bodies_exactly() {
        let mut msg = message(1, 1);
        msg.raw_body = vec![0x1f, 0x8b, 0x08, 0xff, 0x00];
        let fake = FakeEntity::with(vec![msg], None);
        let cancel = AtomicBool::new(false);
        resend_dlq_loop(&fake, &pair(), None, &cancel, &no_progress())
            .await
            .expect("resend");
        fake.snapshot(|st| assert_eq!(st.sent[0].1.body, vec![0x1f, 0x8b, 0x08, 0xff, 0x00]));
    }

    #[tokio::test]
    async fn delete_selected_removes_only_targets() {
        let fake = FakeEntity::with((1..=5).map(|s| message(s, 1)).collect(), Some(10));
        let cancel = AtomicBool::new(false);
        let groups = vec![(
            "q".to_string(),
            Some(10),
            [3].into_iter().collect::<HashSet<i64>>(),
        )];
        let (removed, warnings, cancelled) = delete_selected(&fake, &groups, &cancel)
            .await
            .expect("delete");
        assert_eq!(removed, vec![3]);
        assert!(warnings.is_empty());
        assert!(!cancelled);
        fake.snapshot(|st| assert_eq!(st.available.len(), 4));
    }
}
