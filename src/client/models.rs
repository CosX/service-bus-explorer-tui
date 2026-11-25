use serde::{Deserialize, Serialize};
use std::time::Instant;

// ──────────────────────────── Entity Models ────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QueueDescription {
    pub name: String,
    pub lock_duration: Option<String>,
    pub max_size_in_megabytes: Option<i64>,
    pub requires_duplicate_detection: Option<bool>,
    pub requires_session: Option<bool>,
    pub default_message_time_to_live: Option<String>,
    pub dead_lettering_on_message_expiration: Option<bool>,
    pub duplicate_detection_history_time_window: Option<String>,
    pub max_delivery_count: Option<i32>,
    pub enable_batched_operations: Option<bool>,
    pub status: Option<String>,
    pub forward_to: Option<String>,
    pub forward_dead_lettered_messages_to: Option<String>,
    pub auto_delete_on_idle: Option<String>,
    pub enable_partitioning: Option<bool>,
    pub user_metadata: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QueueRuntimeInfo {
    pub name: String,
    pub active_message_count: i64,
    pub dead_letter_message_count: i64,
    pub scheduled_message_count: i64,
    pub transfer_message_count: i64,
    pub transfer_dead_letter_message_count: i64,
    pub size_in_bytes: i64,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub accessed_at: Option<String>,
    pub message_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TopicDescription {
    pub name: String,
    pub max_size_in_megabytes: Option<i64>,
    pub default_message_time_to_live: Option<String>,
    pub requires_duplicate_detection: Option<bool>,
    pub duplicate_detection_history_time_window: Option<String>,
    pub enable_batched_operations: Option<bool>,
    pub status: Option<String>,
    pub support_ordering: Option<bool>,
    pub auto_delete_on_idle: Option<String>,
    pub enable_partitioning: Option<bool>,
    pub user_metadata: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TopicRuntimeInfo {
    pub name: String,
    pub subscription_count: i64,
    pub active_message_count: i64,
    pub dead_letter_message_count: i64,
    pub size_in_bytes: i64,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub accessed_at: Option<String>,
    pub scheduled_message_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubscriptionDescription {
    pub name: String,
    pub topic_name: String,
    pub lock_duration: Option<String>,
    pub requires_session: Option<bool>,
    pub default_message_time_to_live: Option<String>,
    pub dead_lettering_on_message_expiration: Option<bool>,
    pub dead_lettering_on_filter_evaluation_exceptions: Option<bool>,
    pub max_delivery_count: Option<i32>,
    pub enable_batched_operations: Option<bool>,
    pub status: Option<String>,
    pub forward_to: Option<String>,
    pub forward_dead_lettered_messages_to: Option<String>,
    pub auto_delete_on_idle: Option<String>,
    pub user_metadata: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubscriptionRuntimeInfo {
    pub name: String,
    pub topic_name: String,
    pub active_message_count: i64,
    pub dead_letter_message_count: i64,
    pub transfer_message_count: i64,
    pub transfer_dead_letter_message_count: i64,
    pub message_count: i64,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub accessed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubscriptionRule {
    pub name: String,
    pub sql_expression: String,
}

// ──────────────────────────── Message Models ────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceBusMessage {
    pub body: String,
    pub content_type: Option<String>,
    pub message_id: Option<String>,
    pub correlation_id: Option<String>,
    pub session_id: Option<String>,
    pub label: Option<String>,
    pub to: Option<String>,
    pub reply_to: Option<String>,
    pub time_to_live: Option<String>,
    pub scheduled_enqueue_time: Option<String>,
    pub partition_key: Option<String>,
    pub custom_properties: Vec<(String, String)>,
}

impl Default for ServiceBusMessage {
    fn default() -> Self {
        Self {
            body: String::new(),
            content_type: Some("application/json".to_string()),
            message_id: None,
            correlation_id: None,
            session_id: None,
            label: None,
            to: None,
            reply_to: None,
            time_to_live: None,
            scheduled_enqueue_time: None,
            partition_key: None,
            custom_properties: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceivedMessage {
    pub body: String,
    pub broker_properties: BrokerProperties,
    pub custom_properties: Vec<(String, String)>,
    /// The lock token URI for peek-locked messages (used for complete/abandon/deadletter).
    pub lock_token_uri: Option<String>,
    /// The entity path this message was peeked from (without `/$deadletterqueue`).
    /// Populated during peek so bulk-resend knows which DLQ to remove from,
    /// especially for topic fan-out where messages come from multiple subscription DLQs.
    #[serde(skip)]
    pub source_entity: Option<String>,
}

impl ReceivedMessage {
    /// Convert to a sendable message, preserving body, metadata, and custom properties.
    /// Drops broker-assigned fields (sequence number, enqueued time, delivery count, etc.).
    pub fn to_sendable(&self) -> ServiceBusMessage {
        ServiceBusMessage {
            body: self.body.clone(),
            content_type: self.broker_properties.content_type.clone(),
            message_id: self.broker_properties.message_id.clone(),
            correlation_id: self.broker_properties.correlation_id.clone(),
            session_id: self.broker_properties.session_id.clone(),
            label: self.broker_properties.label.clone(),
            custom_properties: self.custom_properties.clone(),
            ..Default::default()
        }
    }

    /// Parse `body` as JSON when possible; otherwise wrap it as a JSON string.
    fn body_as_json_value(&self) -> serde_json::Value {
        match serde_json::from_str::<serde_json::Value>(&self.body) {
            Ok(value) => value,
            Err(_) => serde_json::Value::String(self.body.clone()),
        }
    }

    /// Serialize broker properties, custom properties, and body as pretty JSON
    /// for clipboard export. Omits internal fields (lock token URI, source entity)
    /// and null broker properties. When `body` is valid JSON, it is embedded as a
    /// structured value; otherwise it remains a string.
    pub fn to_clipboard_json(&self) -> Result<String, serde_json::Error> {
        let mut broker = serde_json::to_value(&self.broker_properties)?;
        if let serde_json::Value::Object(ref mut map) = broker {
            map.retain(|_, v| !v.is_null());
        }

        let custom: serde_json::Map<String, serde_json::Value> = self
            .custom_properties
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
            .collect();

        let export = serde_json::json!({
            "broker_properties": broker,
            "custom_properties": custom,
            "body": self.body_as_json_value(),
        });
        serde_json::to_string_pretty(&export)
    }

    /// Serialize only the message body for clipboard export.
    /// Valid JSON bodies are pretty-printed as structured values; otherwise the
    /// raw body text is returned unchanged.
    pub fn to_clipboard_body(&self) -> Result<String, serde_json::Error> {
        match serde_json::from_str::<serde_json::Value>(&self.body) {
            Ok(value) => serde_json::to_string_pretty(&value),
            Err(_) => Ok(self.body.clone()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BrokerProperties {
    #[serde(rename = "MessageId")]
    pub message_id: Option<String>,
    #[serde(rename = "CorrelationId")]
    pub correlation_id: Option<String>,
    #[serde(rename = "SessionId")]
    pub session_id: Option<String>,
    #[serde(rename = "Label")]
    pub label: Option<String>,
    #[serde(rename = "To")]
    pub to: Option<String>,
    #[serde(rename = "ReplyTo")]
    pub reply_to: Option<String>,
    #[serde(rename = "ContentType")]
    pub content_type: Option<String>,
    #[serde(rename = "SequenceNumber")]
    pub sequence_number: Option<i64>,
    #[serde(rename = "EnqueuedSequenceNumber")]
    pub enqueued_sequence_number: Option<i64>,
    #[serde(rename = "EnqueuedTimeUtc")]
    pub enqueued_time_utc: Option<String>,
    #[serde(rename = "LockedUntilUtc")]
    pub locked_until_utc: Option<String>,
    #[serde(rename = "LockToken")]
    pub lock_token: Option<String>,
    #[serde(rename = "TimeToLive")]
    pub time_to_live: Option<f64>,
    #[serde(rename = "DeliveryCount")]
    pub delivery_count: Option<i32>,
    #[serde(rename = "DeadLetterSource")]
    pub dead_letter_source: Option<String>,
    #[serde(rename = "DeadLetterReason")]
    pub dead_letter_reason: Option<String>,
    #[serde(rename = "DeadLetterErrorDescription")]
    pub dead_letter_error_description: Option<String>,
    #[serde(rename = "State")]
    pub state: Option<String>,
    #[serde(rename = "PartitionKey")]
    pub partition_key: Option<String>,
    #[serde(rename = "ScheduledEnqueueTimeUtc")]
    pub scheduled_enqueue_time_utc: Option<String>,
    #[serde(rename = "Size")]
    pub size: Option<i64>,
}

// ──────────────────────────── Tree / UI Models ────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntityType {
    Namespace,
    QueueFolder,
    TopicFolder,
    Queue,
    Topic,
    Subscription,
    SubscriptionFolder,
    #[allow(dead_code)]
    DeadLetterQueue,
}

#[derive(Debug, Clone)]
pub struct TreeNode {
    pub id: String,
    pub label: String,
    pub entity_type: EntityType,
    pub path: String,
    pub depth: usize,
    pub expanded: bool,
    pub children: Vec<TreeNode>,
    pub message_count: Option<i64>,
    pub dlq_count: Option<i64>,
}

impl TreeNode {
    pub fn new_folder(id: &str, label: &str, entity_type: EntityType, depth: usize) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            entity_type,
            path: String::new(),
            depth,
            expanded: true,
            children: Vec::new(),
            message_count: None,
            dlq_count: None,
        }
    }

    pub fn new_entity(
        id: &str,
        label: &str,
        entity_type: EntityType,
        path: &str,
        depth: usize,
    ) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            entity_type,
            path: path.to_string(),
            depth,
            expanded: false,
            children: Vec::new(),
            message_count: None,
            dlq_count: None,
        }
    }

    /// Collect the IDs of all expanded nodes in this tree.
    pub fn collect_expanded_ids(&self, out: &mut std::collections::HashSet<String>) {
        if self.expanded {
            out.insert(self.id.clone());
        }
        for child in &self.children {
            child.collect_expanded_ids(out);
        }
    }

    /// Apply a previously captured set of expanded IDs to this tree.
    /// Nodes whose ID appears in `expanded_ids` are expanded; all others are collapsed.
    pub fn apply_expanded_ids(&mut self, expanded_ids: &std::collections::HashSet<String>) {
        self.expanded = expanded_ids.contains(&self.id);
        for child in &mut self.children {
            child.apply_expanded_ids(expanded_ids);
        }
    }

    /// Flatten this tree into a displayable list of visible nodes.
    pub fn flatten(&self) -> Vec<FlatNode> {
        let mut result = Vec::new();
        self.flatten_inner(&mut result);
        result
    }

    fn flatten_inner(&self, out: &mut Vec<FlatNode>) {
        out.push(FlatNode {
            id: self.id.clone(),
            label: self.label.clone(),
            entity_type: self.entity_type.clone(),
            path: self.path.clone(),
            depth: self.depth,
            expanded: self.expanded,
            has_children: !self.children.is_empty(),
            message_count: self.message_count,
            dlq_count: self.dlq_count,
        });
        if self.expanded {
            for child in &self.children {
                child.flatten_inner(out);
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct FlatNode {
    pub id: String,
    pub label: String,
    pub entity_type: EntityType,
    pub path: String,
    pub depth: usize,
    pub expanded: bool,
    pub has_children: bool,
    pub message_count: Option<i64>,
    pub dlq_count: Option<i64>,
}

// ──────────────────────────── Azure Monitor Metrics ────────────────────────────

/// Top-level Azure Monitor metrics response.
#[derive(Debug, Deserialize)]
pub struct MetricsResponse {
    pub value: Vec<MetricValue>,
}

/// A single metric from the Azure Monitor response.
#[derive(Debug, Clone, Deserialize)]
pub struct MetricValue {
    pub name: MetricName,
    pub timeseries: Vec<MetricTimeSeries>,
}

/// Metric name descriptor.
#[derive(Debug, Clone, Deserialize)]
pub struct MetricName {
    pub value: String,
}

/// A time series from Azure Monitor.
#[derive(Debug, Clone, Deserialize)]
pub struct MetricTimeSeries {
    pub data: Vec<MetricDataPoint>,
}

/// A single data point from Azure Monitor.
#[derive(Debug, Clone, Deserialize)]
pub struct MetricDataPoint {
    #[serde(rename = "timeStamp")]
    pub timestamp: String,
    pub average: Option<f64>,
    pub total: Option<f64>,
}

/// Processed metrics ready for sparkline rendering.
#[derive(Debug, Clone)]
pub struct EntityMetrics {
    pub active_messages: Vec<u64>,
    pub dead_letter_messages: Vec<u64>,
    pub incoming_messages: Vec<u64>,
    pub outgoing_messages: Vec<u64>,
    pub scheduled_messages: Vec<u64>,
    pub entity_name: String,
    pub fetched_at: Instant,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_json_includes_props_and_body() {
        let msg = ReceivedMessage {
            body: r#"{"hello":"world"}"#.to_string(),
            broker_properties: BrokerProperties {
                message_id: Some("msg-1".to_string()),
                sequence_number: Some(42),
                delivery_count: Some(1),
                ..Default::default()
            },
            custom_properties: vec![("foo".to_string(), "bar".to_string())],
            lock_token_uri: Some("https://example/lock".to_string()),
            source_entity: Some("my-queue".to_string()),
        };

        let json = msg.to_clipboard_json().expect("serialize");
        let value: serde_json::Value = serde_json::from_str(&json).expect("parse");

        // JSON body is embedded as an object, not an escaped string
        assert_eq!(value["body"]["hello"], "world");
        assert!(value["body"].is_object());
        assert_eq!(value["broker_properties"]["MessageId"], "msg-1");
        assert_eq!(value["broker_properties"]["SequenceNumber"], 42);
        assert_eq!(value["custom_properties"]["foo"], "bar");
        // Internal fields must not leak into the clipboard payload
        assert!(value.get("lock_token_uri").is_none());
        assert!(value.get("source_entity").is_none());
        // Null broker props are stripped
        assert!(value["broker_properties"].get("CorrelationId").is_none());
    }

    #[test]
    fn clipboard_json_keeps_non_json_body_as_string() {
        let msg = ReceivedMessage {
            body: "plain text body".to_string(),
            broker_properties: BrokerProperties::default(),
            custom_properties: Vec::new(),
            lock_token_uri: None,
            source_entity: None,
        };

        let json = msg.to_clipboard_json().expect("serialize");
        let value: serde_json::Value = serde_json::from_str(&json).expect("parse");
        assert_eq!(value["body"], "plain text body");
        assert!(value["body"].is_string());
    }

    #[test]
    fn clipboard_body_pretty_prints_json() {
        let msg = ReceivedMessage {
            body: r#"{"hello":"world"}"#.to_string(),
            broker_properties: BrokerProperties::default(),
            custom_properties: Vec::new(),
            lock_token_uri: None,
            source_entity: None,
        };

        let text = msg.to_clipboard_body().expect("serialize");
        let value: serde_json::Value = serde_json::from_str(&text).expect("parse");
        assert_eq!(value["hello"], "world");
        assert!(text.contains('\n'), "expected pretty-printed JSON");
    }

    #[test]
    fn clipboard_body_returns_raw_non_json() {
        let msg = ReceivedMessage {
            body: "not json".to_string(),
            broker_properties: BrokerProperties::default(),
            custom_properties: Vec::new(),
            lock_token_uri: None,
            source_entity: None,
        };

        assert_eq!(msg.to_clipboard_body().expect("serialize"), "not json");
    }

    #[test]
    fn clipboard_json_embeds_array_body() {
        let msg = ReceivedMessage {
            body: r#"[1,{"x":true}]"#.to_string(),
            broker_properties: BrokerProperties::default(),
            custom_properties: Vec::new(),
            lock_token_uri: None,
            source_entity: None,
        };

        let json = msg.to_clipboard_json().expect("serialize");
        let value: serde_json::Value = serde_json::from_str(&json).expect("parse");
        assert!(value["body"].is_array());
        assert_eq!(value["body"][0], 1);
        assert_eq!(value["body"][1]["x"], true);
    }

    #[test]
    fn clipboard_full_and_body_agree_on_parsed_body() {
        let msg = ReceivedMessage {
            body: r#"{"entity":{"id":"1"},"tenantId":"2"}"#.to_string(),
            broker_properties: BrokerProperties {
                correlation_id: Some("corr".to_string()),
                ..Default::default()
            },
            custom_properties: vec![("messagetype".to_string(), "OrderCreated".to_string())],
            lock_token_uri: None,
            source_entity: None,
        };

        let full: serde_json::Value =
            serde_json::from_str(&msg.to_clipboard_json().expect("full")).expect("parse full");
        let body_only: serde_json::Value =
            serde_json::from_str(&msg.to_clipboard_body().expect("body")).expect("parse body");

        assert_eq!(full["body"], body_only);
        assert_eq!(full["broker_properties"]["CorrelationId"], "corr");
        assert_eq!(full["custom_properties"]["messagetype"], "OrderCreated");
    }
}
