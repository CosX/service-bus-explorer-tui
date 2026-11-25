use reqwest::Client;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::auth::ConnectionConfig;
use super::entity_path;
use super::error::{Result, ServiceBusError};
use super::models::*;
use super::scan::{self, LockingReceiver, MessageSender, PeekOutcome, ScanLimits, ScanReport};

/// Per-request timeout. Server-side receive waits are 1s, so anything close to
/// this means a stalled connection, not a slow broker.
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

/// Headers that are part of the HTTP exchange, not Service Bus user
/// properties. Anything else on a received message is a user property.
const NON_PROPERTY_HEADERS: &[&str] = &[
    "brokerproperties",
    "date",
    "server",
    "transfer-encoding",
    "strict-transport-security",
    "location",
    "connection",
    "keep-alive",
    "vary",
    "alt-svc",
    "cache-control",
    "expires",
    "pragma",
    "etag",
    "last-modified",
    "age",
    "via",
    "set-cookie",
    "retry-after",
    "www-authenticate",
    "trailer",
    "upgrade",
    "accept-ranges",
    "request-context",
    "timing-allow-origin",
    "nel",
    "report-to",
    "permissions-policy",
    "referrer-policy",
    "x-content-type-options",
    "x-powered-by",
    "x-frame-options",
    "x-xss-protection",
    "x-aspnet-version",
    "x-cache",
];

/// Header prefixes that are never user properties.
const NON_PROPERTY_PREFIXES: &[&str] = &["content-", "x-ms-", "access-control-", "proxy-"];

/// Header names a user property must not use: they would override the
/// request's authentication, framing or broker metadata.
const RESERVED_SEND_HEADERS: &[&str] = &[
    "authorization",
    "brokerproperties",
    "content-type",
    "content-length",
    "host",
    "transfer-encoding",
    "connection",
];

fn is_user_property_header(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    !NON_PROPERTY_HEADERS.contains(&n.as_str())
        && !NON_PROPERTY_PREFIXES.iter().any(|p| n.starts_with(p))
}

/// Client for Azure Service Bus data-plane operations (send, receive, peek).
#[derive(Clone)]
pub struct DataPlaneClient {
    config: ConnectionConfig,
    http: Client,
}

impl DataPlaneClient {
    pub fn new(config: ConnectionConfig) -> Self {
        let http = Client::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .unwrap_or_default();
        Self { config, http }
    }

    /// Normalize entity paths for the data-plane REST API.
    /// Management API uses `/Subscriptions/` but data plane expects `/subscriptions/`.
    fn normalize_path(entity_path: &str) -> String {
        entity_path::to_data_plane_path(entity_path)
    }

    // ────────── Send ──────────

    /// Send a message to a queue or topic.
    pub async fn send_message(&self, entity_path: &str, message: &ServiceBusMessage) -> Result<()> {
        for (name, _) in &message.custom_properties {
            if RESERVED_SEND_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
                return Err(ServiceBusError::Operation(format!(
                    "'{}' cannot be used as a custom property name",
                    name
                )));
            }
        }

        let entity_path = Self::normalize_path(entity_path);
        let url = format!(
            "{}/{}/messages?api-version=2017-04",
            self.config.endpoint, entity_path
        );
        let token = self.config.entity_token(&entity_path).await?;

        let mut req = self.http.post(&url).header("Authorization", token).header(
            "Content-Type",
            message
                .content_type
                .as_deref()
                .unwrap_or("application/json"),
        );

        // Build BrokerProperties JSON header
        let mut broker_props = serde_json::Map::new();
        let mut put = |key: &str, value: &Option<String>| {
            if let Some(v) = value {
                broker_props.insert(key.into(), Value::String(v.clone()));
            }
        };
        put("MessageId", &message.message_id);
        put("CorrelationId", &message.correlation_id);
        put("SessionId", &message.session_id);
        put("Label", &message.label);
        put("To", &message.to);
        put("ReplyTo", &message.reply_to);
        put("ScheduledEnqueueTimeUtc", &message.scheduled_enqueue_time);
        put("PartitionKey", &message.partition_key);
        if let Some(ref v) = message.time_to_live {
            let secs = v.trim().parse::<f64>().map_err(|_| {
                ServiceBusError::Operation(format!("TTL '{}' is not a number of seconds", v))
            })?;
            broker_props.insert("TimeToLive".into(), Value::from(secs));
        }

        if !broker_props.is_empty() {
            let json = serde_json::to_string(&broker_props)
                .map_err(|e| ServiceBusError::Operation(format!("BrokerProperties: {}", e)))?;
            req = req.header("BrokerProperties", json);
        }

        // User properties travel as headers holding their wire-form literal.
        for (k, v) in &message.custom_properties {
            req = req.header(k.as_str(), v.as_str());
        }

        let resp = req.body(message.body.clone()).send().await?;

        let status = resp.status().as_u16();
        if status >= 400 {
            let body = resp.text().await?;
            return Err(ServiceBusError::Api { status, body });
        }
        Ok(())
    }

    // ────────── Peek ──────────

    /// Peek messages by locking and then releasing them.
    ///
    /// See [`scan::peek`]: each peek costs every returned message one delivery
    /// attempt. Pass the entity's `MaxDeliveryCount` for active entities so the
    /// peek stops before dead-lettering more than one message.
    pub async fn peek_messages(
        &self,
        entity_path: &str,
        count: usize,
        max_delivery_count: Option<i32>,
    ) -> Result<PeekOutcome> {
        scan::peek(self, entity_path, count, max_delivery_count).await
    }

    // ────────── Receive ──────────

    /// Receive and delete a message (destructive).
    ///
    /// Uses `timeout=1` to avoid the 60-second default server-side long-poll
    /// when the entity is empty.
    pub async fn receive_and_delete(&self, entity_path: &str) -> Result<Option<ReceivedMessage>> {
        let entity_path = Self::normalize_path(entity_path);
        let url = format!(
            "{}/{}/messages/head?api-version=2017-04&timeout=1",
            self.config.endpoint, entity_path
        );
        let token = self.config.entity_token(&entity_path).await?;

        let resp = self
            .http
            .delete(&url)
            .header("Authorization", token)
            .send()
            .await?;

        let status = resp.status().as_u16();
        if status == 204 {
            return Ok(None);
        }
        if status >= 400 {
            let body = resp.text().await?;
            return Err(ServiceBusError::Api { status, body });
        }

        let msg = parse_received_message(resp).await?;
        Ok(Some(msg))
    }

    /// Peek-lock a message (non-destructive receive, requires later disposition).
    pub async fn peek_lock(
        &self,
        entity_path: &str,
        timeout_secs: u32,
    ) -> Result<Option<ReceivedMessage>> {
        let entity_path = Self::normalize_path(entity_path);
        let url = format!(
            "{}/{}/messages/head?api-version=2017-04&timeout={}",
            self.config.endpoint, entity_path, timeout_secs
        );
        let token = self.config.entity_token(&entity_path).await?;

        let resp = self
            .http
            .post(&url)
            .header("Authorization", token)
            .header("Content-Length", "0")
            .body("")
            .send()
            .await?;

        let status = resp.status().as_u16();
        if status == 204 {
            return Ok(None);
        }
        if status >= 400 {
            let body = resp.text().await?;
            return Err(ServiceBusError::Api { status, body });
        }

        let lock_uri = resp
            .headers()
            .get("Location")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let mut msg = parse_received_message(resp).await?;
        msg.lock_token_uri = lock_uri;
        Ok(Some(msg))
    }

    /// Complete a peek-locked message (removes it from the queue).
    pub async fn complete_message(&self, lock_token_uri: &str) -> Result<()> {
        let token = self.config.namespace_token().await?;

        let resp = self
            .http
            .delete(lock_token_uri)
            .header("Authorization", token)
            .send()
            .await?;

        let status = resp.status().as_u16();
        if status >= 400 {
            let body = resp.text().await?;
            return Err(ServiceBusError::Api { status, body });
        }
        Ok(())
    }

    /// Abandon a peek-locked message (makes it available again).
    pub async fn abandon_message(&self, lock_token_uri: &str) -> Result<()> {
        let token = self.config.namespace_token().await?;

        let resp = self
            .http
            .put(lock_token_uri)
            .header("Authorization", token)
            .header("Content-Length", "0")
            .body("")
            .send()
            .await?;

        let status = resp.status().as_u16();
        if status >= 400 {
            let body = resp.text().await?;
            return Err(ServiceBusError::Api { status, body });
        }
        Ok(())
    }

    // ────────── Targeted removal ──────────

    /// Remove specific messages from `path` (an entity or its
    /// `/$deadletterqueue`) by sequence number.
    ///
    /// `max_delivery_count` must be the entity's limit when `path` is an
    /// active entity, and `None` for a dead-letter queue.
    pub async fn remove_by_sequence(
        &self,
        path: &str,
        sequence_numbers: &HashSet<i64>,
        max_delivery_count: Option<i32>,
        cancel: Option<&AtomicBool>,
    ) -> Result<ScanReport> {
        let limits = ScanLimits {
            max_scan: scan::DEFAULT_MAX_SCAN,
            max_delivery_count,
        };
        scan::scan_for_sequences(self, path, sequence_numbers, limits, cancel, |_| async {
            Ok(())
        })
        .await
    }

    /// Remove a single message from an entity's DLQ by sequence number.
    /// Returns `true` if it was found and removed.
    pub async fn remove_from_dlq(&self, entity_path: &str, sequence_number: i64) -> Result<bool> {
        let dlq_path = format!("{}/$deadletterqueue", entity_path);
        let targets: HashSet<i64> = [sequence_number].into_iter().collect();
        let report = self
            .remove_by_sequence(&dlq_path, &targets, None, None)
            .await?;
        Ok(report.completed.contains(&sequence_number))
    }

    // ────────── Purge ──────────

    /// Concurrently purge all messages from an entity.
    ///
    /// Spawns `concurrency` parallel receive-and-delete workers that drain the
    /// entity as fast as the broker allows.  Returns the total number of
    /// messages deleted.  The optional `cancel` flag lets the caller abort
    /// early; `progress` (if given) always holds the running total.
    pub async fn purge_concurrent(
        &self,
        entity_path: &str,
        concurrency: usize,
        cancel: Option<Arc<AtomicBool>>,
        progress: Option<Arc<AtomicU64>>,
    ) -> Result<u64> {
        let count = Arc::new(AtomicU64::new(0));
        let done = Arc::new(AtomicBool::new(false));
        let first_error: Arc<tokio::sync::Mutex<Option<ServiceBusError>>> =
            Arc::new(tokio::sync::Mutex::new(None));

        let mut handles = Vec::with_capacity(concurrency);
        for _ in 0..concurrency {
            let dp = self.clone();
            let path = entity_path.to_string();
            let count = Arc::clone(&count);
            let done = Arc::clone(&done);
            let cancel = cancel.clone();
            let progress = progress.clone();
            let first_error = Arc::clone(&first_error);

            handles.push(tokio::spawn(async move {
                loop {
                    if done.load(Ordering::Relaxed) {
                        return;
                    }
                    if cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed)) {
                        return;
                    }
                    match dp.receive_and_delete(&path).await {
                        Ok(Some(_)) => {
                            count.fetch_add(1, Ordering::Relaxed);
                            if let Some(ref p) = progress {
                                p.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        Ok(None) => {
                            done.store(true, Ordering::Relaxed);
                            return;
                        }
                        Err(e) => {
                            done.store(true, Ordering::Relaxed);
                            let mut guard = first_error.lock().await;
                            if guard.is_none() {
                                *guard = Some(e);
                            }
                            return;
                        }
                    }
                }
            }));
        }

        for h in handles {
            let _ = h.await;
        }

        let err = first_error.lock().await.take();
        if let Some(e) = err {
            return Err(e);
        }

        Ok(count.load(Ordering::Relaxed))
    }
}

#[async_trait::async_trait]
impl LockingReceiver for DataPlaneClient {
    async fn lock_next(&self, path: &str) -> Result<Option<ReceivedMessage>> {
        self.peek_lock(path, 1).await
    }

    async fn complete(&self, lock_uri: &str) -> Result<()> {
        self.complete_message(lock_uri).await
    }

    async fn release(&self, lock_uri: &str) -> Result<()> {
        self.abandon_message(lock_uri).await
    }
}

#[async_trait::async_trait]
impl MessageSender for DataPlaneClient {
    async fn send(&self, path: &str, message: &ServiceBusMessage) -> Result<()> {
        self.send_message(path, message).await
    }
}

// ──────────────────────────── Response parsing ────────────────────────────

async fn parse_received_message(resp: reqwest::Response) -> Result<ReceivedMessage> {
    let broker_props_str = resp
        .headers()
        .get("BrokerProperties")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("{}")
        .to_string();

    // User properties arrive as headers holding wire-form literals
    // (`"text"`, `42`, `true`); keep them verbatim so a resend preserves type.
    let raw_props: Vec<(String, String)> = resp
        .headers()
        .iter()
        .filter(|(name, _)| is_user_property_header(name.as_str()))
        .map(|(name, value)| {
            (
                name.to_string(),
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            )
        })
        .collect();

    let body = resp.bytes().await?.to_vec();

    let broker_properties: BrokerProperties =
        serde_json::from_str(&broker_props_str).unwrap_or_default();

    Ok(ReceivedMessage::new(body, broker_properties, raw_props))
}

#[cfg(test)]
mod tests {
    use super::is_user_property_header;

    #[test]
    fn transport_headers_are_not_user_properties() {
        for h in [
            "Connection",
            "Vary",
            "Alt-Svc",
            "Content-Type",
            "x-ms-request-id",
            "X-Content-Type-Options",
            "BrokerProperties",
            "Location",
        ] {
            assert!(!is_user_property_header(h), "{} must be filtered", h);
        }
    }

    #[test]
    fn application_headers_are_user_properties() {
        for h in ["tenant", "DeadLetterReason", "orderId", "x-tenant"] {
            assert!(is_user_property_header(h), "{} must be kept", h);
        }
    }
}
