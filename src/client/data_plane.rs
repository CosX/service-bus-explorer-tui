use reqwest::Client;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use super::auth::ConnectionConfig;
use super::error::{Result, ServiceBusError};
use super::models::*;

/// Client for Azure Service Bus data-plane operations (send, receive, peek).
#[derive(Clone)]
pub struct DataPlaneClient {
    config: ConnectionConfig,
    http: Client,
}

impl DataPlaneClient {
    pub fn new(config: ConnectionConfig) -> Self {
        Self {
            config,
            http: Client::new(),
        }
    }

    /// Normalize entity paths for the data-plane REST API.
    /// Management API uses `/Subscriptions/` but data plane expects `/subscriptions/`.
    fn normalize_path(entity_path: &str) -> String {
        entity_path.replace("/Subscriptions/", "/subscriptions/")
    }

    // ────────── Send ──────────

    /// Send a message to a queue or topic.
    pub async fn send_message(
        &self,
        entity_path: &str,
        message: &ServiceBusMessage,
    ) -> Result<()> {
        let entity_path = Self::normalize_path(entity_path);
        let url = format!(
            "{}/{}/messages?api-version=2017-04",
            self.config.endpoint, entity_path
        );
        let token = self.config.entity_token(&entity_path).await?;

        let mut req = self
            .http
            .post(&url)
            .header("Authorization", token)
            .header(
                "Content-Type",
                message
                    .content_type
                    .as_deref()
                    .unwrap_or("application/json"),
            );

        // Build BrokerProperties JSON header
        let mut broker_props = serde_json::Map::new();
        if let Some(ref id) = message.message_id {
            broker_props.insert("MessageId".into(), Value::String(id.clone()));
        }
        if let Some(ref id) = message.correlation_id {
            broker_props.insert("CorrelationId".into(), Value::String(id.clone()));
        }
        if let Some(ref id) = message.session_id {
            broker_props.insert("SessionId".into(), Value::String(id.clone()));
        }
        if let Some(ref v) = message.label {
            broker_props.insert("Label".into(), Value::String(v.clone()));
        }
        if let Some(ref v) = message.to {
            broker_props.insert("To".into(), Value::String(v.clone()));
        }
        if let Some(ref v) = message.reply_to {
            broker_props.insert("ReplyTo".into(), Value::String(v.clone()));
        }
        if let Some(ref v) = message.time_to_live {
            if let Ok(secs) = v.parse::<f64>() {
                broker_props.insert("TimeToLive".into(), Value::from(secs));
            }
        }
        if let Some(ref v) = message.scheduled_enqueue_time {
            broker_props.insert(
                "ScheduledEnqueueTimeUtc".into(),
                Value::String(v.clone()),
            );
        }
        if let Some(ref v) = message.partition_key {
            broker_props.insert("PartitionKey".into(), Value::String(v.clone()));
        }

        if !broker_props.is_empty() {
            req = req.header(
                "BrokerProperties",
                serde_json::to_string(&broker_props).unwrap_or_default(),
            );
        }

        // Custom properties as individual headers
        for (k, v) in &message.custom_properties {
            req = req.header(k.as_str(), format!("\"{}\"", v));
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

    /// Peek messages without permanently removing them.
    ///
    /// Uses peek-lock + abandon under the hood because the REST API's
    /// `PeekOnly=true` has no cursor and always returns the same first message.
    /// Peek-lock gives us unique messages; we abandon all locks afterward so
    /// messages remain available. Note: each peek-lock increments `DeliveryCount`.
    pub async fn peek_messages(
        &self,
        entity_path: &str,
        count: i32,
    ) -> Result<Vec<ReceivedMessage>> {
        let mut messages = Vec::new();
        let mut lock_uris = Vec::new();

        for _ in 0..count {
            match self.peek_lock(entity_path, 1).await? {
                Some(msg) => {
                    if let Some(ref uri) = msg.lock_token_uri {
                        lock_uris.push(uri.clone());
                    }
                    messages.push(msg);
                }
                None => break,
            }
        }

        // Abandon all locks — messages become available again.
        for uri in &lock_uris {
            let _ = self.abandon_message(uri).await;
        }

        // Clear lock URIs from returned messages (locks are released)
        for msg in &mut messages {
            msg.lock_token_uri = None;
        }

        Ok(messages)
    }

    // ────────── Receive ──────────

    /// Receive and delete a message (destructive).
    ///
    /// Uses `timeout=1` to avoid the 60-second default server-side long-poll
    /// when the entity is empty.
    pub async fn receive_and_delete(
        &self,
        entity_path: &str,
    ) -> Result<Option<ReceivedMessage>> {
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

    /// Dead-letter a peek-locked message.
    pub async fn dead_letter_message(
        &self,
        lock_token_uri: &str,
        reason: Option<&str>,
        description: Option<&str>,
    ) -> Result<()> {
        let token = self.config.namespace_token().await?;

        let mut props = serde_json::Map::new();
        props.insert(
            "DeadLetterReason".into(),
            Value::String(reason.unwrap_or("Manual dead-letter").to_string()),
        );
        if let Some(desc) = description {
            props.insert(
                "DeadLetterErrorDescription".into(),
                Value::String(desc.to_string()),
            );
        }

        let resp = self
            .http
            .put(lock_token_uri)
            .header("Authorization", token)
            .header("BrokerProperties", serde_json::to_string(&props).unwrap())
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

    // ────────── Single-message removal ──────────

    /// Remove a specific message from the DLQ by sequence number.
    ///
    /// Peek-locks messages one at a time, looking for a matching sequence number.
    /// Completes the match and abandons any non-matching messages that were locked
    /// along the way.  Returns `true` if the message was found and removed.
    pub async fn remove_from_dlq(
        &self,
        entity_path: &str,
        sequence_number: i64,
    ) -> Result<bool> {
        let dlq_path = format!("{}/$deadletterqueue", entity_path);
        let mut abandoned_uris: Vec<String> = Vec::new();
        let max_attempts = 50u32;

        for _ in 0..max_attempts {
            match self.peek_lock(&dlq_path, 1).await? {
                Some(msg) => {
                    let lock_uri = match msg.lock_token_uri {
                        Some(ref uri) => uri.clone(),
                        None => continue,
                    };

                    if msg.broker_properties.sequence_number == Some(sequence_number) {
                        // Found it — complete (delete from DLQ)
                        self.complete_message(&lock_uri).await?;
                        // Abandon everything else we locked
                        for uri in &abandoned_uris {
                            let _ = self.abandon_message(uri).await;
                        }
                        return Ok(true);
                    } else {
                        // Not a match — will abandon after we're done
                        abandoned_uris.push(lock_uri);
                    }
                }
                None => break,
            }
        }

        // Didn't find it — abandon all locks
        for uri in &abandoned_uris {
            let _ = self.abandon_message(uri).await;
        }
        Ok(false)
    }

    // ────────── Purge ──────────

    /// Concurrently purge all messages from an entity.
    ///
    /// Spawns `concurrency` parallel receive-and-delete workers that drain the
    /// entity as fast as the broker allows.  Returns the total number of
    /// messages deleted.  The optional `cancel` flag lets the caller abort
    /// early; the optional `progress` callback is invoked after every message.
    pub async fn purge_concurrent(
        &self,
        entity_path: &str,
        concurrency: usize,
        cancel: Option<Arc<AtomicBool>>,
        progress: Option<tokio::sync::mpsc::UnboundedSender<u64>>,
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
                    if let Some(ref c) = cancel {
                        if c.load(Ordering::Relaxed) {
                            return;
                        }
                    }
                    match dp.receive_and_delete(&path).await {
                        Ok(Some(_)) => {
                            let n = count.fetch_add(1, Ordering::Relaxed) + 1;
                            if let Some(ref tx) = progress {
                                let _ = tx.send(n);
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

// ──────────────────────────── Response parsing ────────────────────────────

async fn parse_received_message(resp: reqwest::Response) -> Result<ReceivedMessage> {
    let broker_props_str = resp
        .headers()
        .get("BrokerProperties")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("{}")
        .to_string();

    // Collect custom properties from headers (all non-standard headers)
    let custom_props: Vec<(String, String)> = resp
        .headers()
        .iter()
        .filter(|(name, _)| {
            let n = name.as_str().to_lowercase();
            !n.starts_with("content-")
                && n != "brokerproperties"
                && n != "date"
                && n != "server"
                && n != "transfer-encoding"
                && n != "strict-transport-security"
                && n != "location"
                && n != "x-ms-request-id"
                && !n.starts_with("x-ms-")
        })
        .map(|(name, value)| {
            (
                name.to_string(),
                value
                    .to_str()
                    .unwrap_or("")
                    .trim_matches('"')
                    .to_string(),
            )
        })
        .collect();

    let body = resp.text().await?;

    let broker_properties: BrokerProperties =
        serde_json::from_str(&broker_props_str).unwrap_or_default();

    Ok(ReceivedMessage {
        body,
        broker_properties,
        custom_properties: custom_props,
        lock_token_uri: None,
    })
}
