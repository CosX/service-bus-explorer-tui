//! Lock-based message algorithms (peek, find-by-sequence-number), written
//! against small transport traits so their termination and lock-release
//! behaviour can be tested without a broker.
//!
//! Every lock taken here is released on every exit path, including errors and
//! cancellation; a leaked lock hides the message for the entity's lock
//! duration and still costs it a delivery attempt.

use std::collections::HashSet;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};

use super::error::Result;
use super::models::{ReceivedMessage, ServiceBusMessage};

/// Peek-lock style receive with explicit settlement.
#[async_trait::async_trait]
pub trait LockingReceiver: Send + Sync {
    /// Lock the next available message, or `None` when the entity is empty.
    async fn lock_next(&self, path: &str) -> Result<Option<ReceivedMessage>>;
    /// Settle a locked message, removing it from the entity.
    async fn complete(&self, lock_uri: &str) -> Result<()>;
    /// Release a lock so the message becomes available again.
    ///
    /// The broker counts this as a delivery attempt; on an entity with a
    /// `MaxDeliveryCount`, releasing a message that has reached the limit
    /// dead-letters it.
    async fn release(&self, lock_uri: &str) -> Result<()>;
}

/// Message send.
#[async_trait::async_trait]
pub trait MessageSender: Send + Sync {
    async fn send(&self, path: &str, message: &ServiceBusMessage) -> Result<()>;
}

/// True when releasing this locked message will make the broker dead-letter
/// it, because its delivery count has reached the entity's limit.
pub fn release_would_dead_letter(msg: &ReceivedMessage, max_delivery_count: Option<i32>) -> bool {
    match (max_delivery_count, msg.broker_properties.delivery_count) {
        (Some(max), Some(count)) => max > 0 && count >= max,
        _ => false,
    }
}

/// Release every held lock, returning how many releases failed.
async fn release_all<R: LockingReceiver + ?Sized>(receiver: &R, held: &[String]) -> u32 {
    let mut failures = 0;
    for uri in held {
        if let Err(e) = receiver.release(uri).await {
            tracing::warn!("failed to release message lock: {}", e);
            failures += 1;
        }
    }
    failures
}

// ──────────────────────────── Peek ────────────────────────────

/// Result of a lock-and-release peek.
#[derive(Debug, Default)]
pub struct PeekOutcome {
    pub messages: Vec<ReceivedMessage>,
    /// Sequence numbers the peek pushed into the dead-letter queue: their
    /// delivery count had reached `MaxDeliveryCount`, so releasing the lock
    /// dead-lettered them. The peek stops at the first one.
    pub dead_lettered_by_peek: Vec<i64>,
    /// Locks that could not be released; those messages stay hidden until
    /// their lock expires.
    pub release_failures: u32,
}

/// Peek up to `count` messages by locking them one at a time, then releasing
/// every lock.
///
/// The REST API's non-locking peek has no cursor, so locking is the only way
/// to see past the first message. Each lock costs the message one delivery
/// attempt. When `max_delivery_count` is known (active queues and
/// subscriptions; dead-letter queues have no limit), the peek stops at the
/// first message whose release will dead-letter it, so a backlog of
/// near-limit messages loses at most one to the peek.
pub async fn peek<R: LockingReceiver + ?Sized>(
    receiver: &R,
    path: &str,
    count: usize,
    max_delivery_count: Option<i32>,
) -> Result<PeekOutcome> {
    let mut outcome = PeekOutcome::default();
    let mut held = Vec::new();

    for _ in 0..count {
        let msg = match receiver.lock_next(path).await {
            Ok(Some(msg)) => msg,
            Ok(None) => break,
            Err(e) => {
                release_all(receiver, &held).await;
                return Err(e);
            }
        };

        if let Some(ref uri) = msg.lock_token_uri {
            held.push(uri.clone());
        }
        let at_limit = release_would_dead_letter(&msg, max_delivery_count);
        if at_limit {
            if let Some(seq) = msg.broker_properties.sequence_number {
                outcome.dead_lettered_by_peek.push(seq);
            }
        }
        outcome.messages.push(msg);
        if at_limit {
            break;
        }
    }

    outcome.release_failures = release_all(receiver, &held).await;
    for msg in &mut outcome.messages {
        msg.lock_token_uri = None;
    }
    Ok(outcome)
}

// ──────────────────────────── Find by sequence number ────────────────────────────

/// Bounds for a sequence-number scan.
#[derive(Debug, Clone, Copy)]
pub struct ScanLimits {
    /// Maximum messages to lock while looking for the targets.
    pub max_scan: usize,
    /// The entity's `MaxDeliveryCount`; `None` for dead-letter queues.
    pub max_delivery_count: Option<i32>,
}

/// Scan bound used for user-initiated removals and resends.
pub const DEFAULT_MAX_SCAN: usize = 1000;

/// What happened during a sequence-number scan.
#[derive(Debug, Default)]
pub struct ScanReport {
    /// Targets acted on and removed from the entity.
    pub completed: Vec<i64>,
    /// Targets whose action failed; they were released and remain in place.
    pub failed: Vec<(i64, String)>,
    /// Targets whose action succeeded but whose removal failed. For a resend
    /// this means the message now exists twice.
    pub complete_failed: Vec<(i64, String)>,
    /// Targets not found within the scan bounds.
    pub not_found: Vec<i64>,
    /// Non-target messages dead-lettered because the scan released them at
    /// their delivery limit. The scan stops at the first one.
    pub dead_lettered_by_scan: Vec<i64>,
    pub scanned: usize,
    pub release_failures: u32,
    pub cancelled: bool,
}

impl ScanReport {
    /// One-line summary of anything that did not go to plan, for the status bar.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.failed.is_empty() {
            let first = &self.failed[0].1;
            out.push(format!("{} failed ({})", self.failed.len(), first));
        }
        if !self.complete_failed.is_empty() {
            out.push(format!(
                "{} sent but could not be removed (now duplicated)",
                self.complete_failed.len()
            ));
        }
        if !self.not_found.is_empty() {
            out.push(format!(
                "{} not found within {} scanned",
                self.not_found.len(),
                self.scanned
            ));
        }
        if !self.dead_lettered_by_scan.is_empty() {
            out.push(format!(
                "stopped: seq {} was at its delivery limit and was dead-lettered by the scan",
                self.dead_lettered_by_scan
                    .iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        if self.release_failures > 0 {
            out.push(format!("{} lock(s) not released", self.release_failures));
        }
        if self.cancelled {
            out.push("cancelled".to_string());
        }
        out
    }
}

/// Lock messages from `path` one at a time until every sequence number in
/// `targets` has been handled, the entity is exhausted, the scan bound is hit,
/// or the scan cycles back to a message it already saw (its lock expired).
///
/// For each target, `action` runs first; on success the message is completed
/// (removed), on failure it is released. Non-target messages are held until
/// the scan ends and then released, so the scan always moves forward.
pub async fn scan_for_sequences<R, F, Fut>(
    receiver: &R,
    path: &str,
    targets: &HashSet<i64>,
    limits: ScanLimits,
    cancel: Option<&AtomicBool>,
    mut action: F,
) -> Result<ScanReport>
where
    R: LockingReceiver + ?Sized,
    F: FnMut(ReceivedMessage) -> Fut,
    Fut: Future<Output = std::result::Result<(), String>>,
{
    let mut report = ScanReport::default();
    let mut remaining: HashSet<i64> = targets.clone();
    let mut seen: HashSet<i64> = HashSet::new();
    let mut held: Vec<String> = Vec::new();

    while !remaining.is_empty() && report.scanned < limits.max_scan {
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            report.cancelled = true;
            break;
        }

        let msg = match receiver.lock_next(path).await {
            Ok(Some(msg)) => msg,
            Ok(None) => break,
            Err(e) => {
                release_all(receiver, &held).await;
                return Err(e);
            }
        };
        report.scanned += 1;

        let Some(uri) = msg.lock_token_uri.clone() else {
            // Without a lock URI the message cannot be settled or released.
            continue;
        };
        let seq = msg.broker_properties.sequence_number;

        // Seeing a message twice means an earlier lock expired and the scan
        // has wrapped around; continuing would loop.
        if let Some(seq) = seq {
            if !seen.insert(seq) {
                held.push(uri);
                break;
            }
        }

        match seq.filter(|s| remaining.contains(s)) {
            Some(target) => {
                remaining.remove(&target);
                match action(msg).await {
                    Ok(()) => match receiver.complete(&uri).await {
                        Ok(()) => report.completed.push(target),
                        Err(e) => report.complete_failed.push((target, e.to_string())),
                    },
                    Err(e) => {
                        report.failed.push((target, e));
                        held.push(uri);
                    }
                }
            }
            None => {
                let at_limit = release_would_dead_letter(&msg, limits.max_delivery_count);
                held.push(uri);
                if at_limit {
                    if let Some(seq) = seq {
                        report.dead_lettered_by_scan.push(seq);
                    }
                    break;
                }
            }
        }
    }

    report.release_failures = release_all(receiver, &held).await;
    let mut not_found: Vec<i64> = remaining.into_iter().collect();
    not_found.sort_unstable();
    report.not_found = not_found;
    Ok(report)
}

// ──────────────────────────── Tests ────────────────────────────

#[cfg(test)]
pub(crate) mod fake {
    //! In-memory broker entity with peek-lock semantics close enough to
    //! Service Bus for termination and settlement tests.

    use std::collections::{HashMap, VecDeque};
    use std::sync::Mutex;

    use super::*;
    use crate::client::error::ServiceBusError;
    use crate::client::models::BrokerProperties;

    #[derive(Default)]
    pub struct State {
        pub available: VecDeque<ReceivedMessage>,
        pub locked: HashMap<String, ReceivedMessage>,
        pub dead_lettered: Vec<i64>,
        pub sent: Vec<(String, ServiceBusMessage)>,
        pub next_lock: u64,
        pub max_delivery_count: Option<i32>,
        /// Send fails while this is true.
        pub send_fails: bool,
        /// Send fails for these message ids.
        pub send_fails_for: Vec<String>,
        pub lock_fails_after: Option<usize>,
        pub locks_taken: usize,
    }

    #[derive(Default)]
    pub struct FakeEntity {
        pub state: Mutex<State>,
    }

    pub fn message(seq: i64, delivery_count: i32) -> ReceivedMessage {
        let bp = BrokerProperties {
            sequence_number: Some(seq),
            delivery_count: Some(delivery_count),
            message_id: Some(format!("m{}", seq)),
            ..Default::default()
        };
        ReceivedMessage::new(format!("body {}", seq).into_bytes(), bp, Vec::new())
    }

    impl FakeEntity {
        pub fn with(messages: Vec<ReceivedMessage>, max_delivery_count: Option<i32>) -> Self {
            let fake = FakeEntity::default();
            {
                let mut st = fake.state.lock().expect("lock");
                st.available = messages.into();
                st.max_delivery_count = max_delivery_count;
            }
            fake
        }

        pub fn snapshot<T>(&self, f: impl FnOnce(&State) -> T) -> T {
            f(&self.state.lock().expect("lock"))
        }
    }

    #[async_trait::async_trait]
    impl LockingReceiver for FakeEntity {
        async fn lock_next(&self, _path: &str) -> Result<Option<ReceivedMessage>> {
            let mut st = self.state.lock().expect("lock");
            if st.lock_fails_after.is_some_and(|n| st.locks_taken >= n) {
                return Err(ServiceBusError::Operation("lock failed".into()));
            }
            let Some(mut msg) = st.available.pop_front() else {
                return Ok(None);
            };
            st.locks_taken += 1;
            st.next_lock += 1;
            let uri = format!("lock-{}", st.next_lock);
            msg.lock_token_uri = Some(uri.clone());
            st.locked.insert(uri, msg.clone());
            Ok(Some(msg))
        }

        async fn complete(&self, lock_uri: &str) -> Result<()> {
            let mut st = self.state.lock().expect("lock");
            st.locked
                .remove(lock_uri)
                .map(|_| ())
                .ok_or_else(|| ServiceBusError::Operation("lock lost".into()))
        }

        async fn release(&self, lock_uri: &str) -> Result<()> {
            let mut st = self.state.lock().expect("lock");
            let Some(mut msg) = st.locked.remove(lock_uri) else {
                return Err(ServiceBusError::Operation("lock lost".into()));
            };
            let count = msg.broker_properties.delivery_count.unwrap_or(1);
            if st.max_delivery_count.is_some_and(|max| count >= max) {
                let seq = msg.broker_properties.sequence_number.unwrap_or(-1);
                st.dead_lettered.push(seq);
                return Ok(());
            }
            msg.broker_properties.delivery_count = Some(count + 1);
            msg.lock_token_uri = None;
            // Released messages return to the head, as on the real broker.
            st.available.push_front(msg);
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl MessageSender for FakeEntity {
        async fn send(&self, path: &str, message: &ServiceBusMessage) -> Result<()> {
            let mut st = self.state.lock().expect("lock");
            let id = message.message_id.clone().unwrap_or_default();
            if st.send_fails || st.send_fails_for.contains(&id) {
                return Err(ServiceBusError::Api {
                    status: 403,
                    body: "send denied".into(),
                });
            }
            st.sent.push((path.to_string(), message.clone()));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{message, FakeEntity};
    use super::*;

    fn seqs(msgs: &[ReceivedMessage]) -> Vec<i64> {
        msgs.iter()
            .filter_map(|m| m.broker_properties.sequence_number)
            .collect()
    }

    #[tokio::test]
    async fn peek_returns_distinct_messages_and_releases_every_lock() {
        let fake = FakeEntity::with((1..=5).map(|s| message(s, 1)).collect(), Some(10));
        let out = peek(&fake, "q", 3, Some(10)).await.expect("peek");
        assert_eq!(seqs(&out.messages), vec![1, 2, 3]);
        assert!(out.messages.iter().all(|m| m.lock_token_uri.is_none()));
        fake.snapshot(|st| {
            assert!(st.locked.is_empty(), "all locks released");
            assert_eq!(st.available.len(), 5);
        });
    }

    #[tokio::test]
    async fn peek_stops_at_the_first_message_at_its_delivery_limit() {
        let msgs = vec![message(1, 2), message(2, 10), message(3, 10), message(4, 1)];
        let fake = FakeEntity::with(msgs, Some(10));
        let out = peek(&fake, "q", 4, Some(10)).await.expect("peek");
        assert_eq!(seqs(&out.messages), vec![1, 2]);
        assert_eq!(out.dead_lettered_by_peek, vec![2]);
        // Only the one at-limit message was lost; seq 3 was never touched.
        fake.snapshot(|st| assert_eq!(st.dead_lettered, vec![2]));
    }

    #[tokio::test]
    async fn peek_on_a_dead_letter_queue_has_no_delivery_limit() {
        let fake = FakeEntity::with(vec![message(1, 50), message(2, 50)], None);
        let out = peek(&fake, "q/$deadletterqueue", 5, None)
            .await
            .expect("peek");
        assert_eq!(out.messages.len(), 2);
        assert!(out.dead_lettered_by_peek.is_empty());
    }

    #[tokio::test]
    async fn peek_releases_held_locks_when_a_later_lock_fails() {
        let fake = FakeEntity::with((1..=5).map(|s| message(s, 1)).collect(), None);
        fake.state.lock().expect("lock").lock_fails_after = Some(2);
        assert!(peek(&fake, "q", 5, None).await.is_err());
        fake.snapshot(|st| assert!(st.locked.is_empty(), "no leaked locks"));
    }

    #[tokio::test]
    async fn scan_completes_targets_and_releases_everything_else() {
        let fake = FakeEntity::with((1..=6).map(|s| message(s, 1)).collect(), None);
        let targets: HashSet<i64> = [2, 5].into_iter().collect();
        let limits = ScanLimits {
            max_scan: 100,
            max_delivery_count: None,
        };
        let report = scan_for_sequences(&fake, "q", &targets, limits, None, |_| async { Ok(()) })
            .await
            .expect("scan");
        assert_eq!(report.completed, vec![2, 5]);
        assert!(report.not_found.is_empty());
        fake.snapshot(|st| {
            assert!(st.locked.is_empty());
            let mut left: Vec<i64> = st
                .available
                .iter()
                .filter_map(|m| m.broker_properties.sequence_number)
                .collect();
            left.sort_unstable();
            assert_eq!(left, vec![1, 3, 4, 6]);
        });
    }

    #[tokio::test]
    async fn scan_reports_targets_it_could_not_find() {
        let fake = FakeEntity::with((1..=3).map(|s| message(s, 1)).collect(), None);
        let targets: HashSet<i64> = [2, 99].into_iter().collect();
        let limits = ScanLimits {
            max_scan: 100,
            max_delivery_count: None,
        };
        let report = scan_for_sequences(&fake, "q", &targets, limits, None, |_| async { Ok(()) })
            .await
            .expect("scan");
        assert_eq!(report.completed, vec![2]);
        assert_eq!(report.not_found, vec![99]);
    }

    #[tokio::test]
    async fn failed_action_leaves_the_target_in_place() {
        let fake = FakeEntity::with((1..=3).map(|s| message(s, 1)).collect(), None);
        let targets: HashSet<i64> = [1, 2].into_iter().collect();
        let limits = ScanLimits {
            max_scan: 100,
            max_delivery_count: None,
        };
        let report = scan_for_sequences(&fake, "q", &targets, limits, None, |m| {
            let fail = m.broker_properties.sequence_number == Some(1);
            async move {
                if fail {
                    Err("boom".to_string())
                } else {
                    Ok(())
                }
            }
        })
        .await
        .expect("scan");
        assert_eq!(report.completed, vec![2]);
        assert_eq!(report.failed.len(), 1);
        fake.snapshot(|st| {
            assert!(st
                .available
                .iter()
                .any(|m| m.broker_properties.sequence_number == Some(1)));
        });
    }

    #[tokio::test]
    async fn scan_stops_before_dead_lettering_more_than_one_bystander() {
        let msgs = vec![message(1, 10), message(2, 10), message(3, 1)];
        let fake = FakeEntity::with(msgs, Some(10));
        let targets: HashSet<i64> = [3].into_iter().collect();
        let limits = ScanLimits {
            max_scan: 100,
            max_delivery_count: Some(10),
        };
        let report = scan_for_sequences(&fake, "q", &targets, limits, None, |_| async { Ok(()) })
            .await
            .expect("scan");
        assert_eq!(report.dead_lettered_by_scan, vec![1]);
        assert_eq!(report.not_found, vec![3]);
        fake.snapshot(|st| assert_eq!(st.dead_lettered, vec![1]));
    }

    #[tokio::test]
    async fn scan_honours_its_bound() {
        let fake = FakeEntity::with((1..=50).map(|s| message(s, 1)).collect(), None);
        let targets: HashSet<i64> = [40].into_iter().collect();
        let limits = ScanLimits {
            max_scan: 10,
            max_delivery_count: None,
        };
        let report = scan_for_sequences(&fake, "q", &targets, limits, None, |_| async { Ok(()) })
            .await
            .expect("scan");
        assert_eq!(report.scanned, 10);
        assert_eq!(report.not_found, vec![40]);
        fake.snapshot(|st| assert!(st.locked.is_empty()));
    }
}
