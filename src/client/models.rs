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
    /// SQL expression for SQL/True/False filters; empty for other kinds.
    pub sql_expression: String,
    pub filter_kind: RuleFilterKind,
}

/// The kind of filter a subscription rule uses.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum RuleFilterKind {
    Sql,
    True,
    False,
    /// Matches on message properties; has no SQL form and cannot be edited here.
    Correlation,
    #[default]
    Unknown,
}

impl RuleFilterKind {
    /// Whether the filter can be shown and edited as a SQL expression.
    pub fn is_sql_editable(self) -> bool {
        matches!(
            self,
            RuleFilterKind::Sql | RuleFilterKind::True | RuleFilterKind::False
        )
    }
}

// ──────────────────────────── Message Models ────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceBusMessage {
    /// Raw body bytes. Kept as bytes so binary payloads survive a resend intact.
    pub body: Vec<u8>,
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
    /// User properties as `(name, header literal)`.
    ///
    /// The literal is sent verbatim as the header value, so it must already be
    /// in Service Bus wire form: a JSON string (`"abc"`), number, or boolean.
    /// Use [`property_literal_from_text`] for user-entered text.
    pub custom_properties: Vec<(String, String)>,
}

impl Default for ServiceBusMessage {
    fn default() -> Self {
        Self {
            body: Vec::new(),
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

/// Encode user-entered text as a string-typed user property literal.
pub fn property_literal_from_text(text: &str) -> String {
    // serde_json escapes quotes and control characters correctly.
    serde_json::to_string(text).unwrap_or_else(|_| format!("\"{}\"", text.replace('"', "")))
}

/// Human-readable form of a property literal: string literals are unquoted,
/// numbers and booleans are shown as-is.
pub fn property_display_value(literal: &str) -> String {
    let trimmed = literal.trim();
    if trimmed.starts_with('"') {
        if let Ok(serde_json::Value::String(s)) = serde_json::from_str(trimmed) {
            return s;
        }
        return trimmed.trim_matches('"').to_string();
    }
    trimmed.to_string()
}

/// User properties the broker attaches when dead-lettering. They describe the
/// old failure and must not travel with a resent copy.
const DEAD_LETTER_PROPERTIES: [&str; 2] = ["DeadLetterReason", "DeadLetterErrorDescription"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceivedMessage {
    /// Body decoded for display and editing. Lossy if the payload is not UTF-8;
    /// see [`ReceivedMessage::body_is_text`].
    pub body: String,
    /// The exact body bytes received from the broker.
    #[serde(skip)]
    pub raw_body: Vec<u8>,
    pub broker_properties: BrokerProperties,
    /// User properties as `(name, display value)` for rendering and search.
    pub custom_properties: Vec<(String, String)>,
    /// User properties as `(name, header literal)`, preserving their wire type.
    #[serde(skip)]
    pub raw_custom_properties: Vec<(String, String)>,
    /// The lock token URI for peek-locked messages (used for complete/abandon/deadletter).
    pub lock_token_uri: Option<String>,
    /// The entity path this message was peeked from (without `/$deadletterqueue`).
    /// Populated during peek so bulk-resend knows which DLQ to remove from,
    /// especially for topic fan-out where messages come from multiple subscription DLQs.
    #[serde(skip)]
    pub source_entity: Option<String>,
}

impl ReceivedMessage {
    /// Build from the raw body bytes and wire-form user properties.
    pub fn new(
        raw_body: Vec<u8>,
        broker_properties: BrokerProperties,
        raw_custom_properties: Vec<(String, String)>,
    ) -> Self {
        let body = String::from_utf8_lossy(&raw_body).into_owned();
        let custom_properties = raw_custom_properties
            .iter()
            .map(|(k, v)| (k.clone(), property_display_value(v)))
            .collect();
        Self {
            body,
            raw_body,
            broker_properties,
            custom_properties,
            raw_custom_properties,
            lock_token_uri: None,
            source_entity: None,
        }
    }

    /// True when the body is valid UTF-8, so editing it as text is lossless.
    pub fn body_is_text(&self) -> bool {
        std::str::from_utf8(&self.raw_body).is_ok()
    }

    /// Wire-form literal of a user property, if present.
    pub fn raw_property(&self, name: &str) -> Option<&str> {
        self.raw_custom_properties
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// Convert to a sendable message, preserving the exact body bytes, metadata
    /// and user properties (with their original types).
    ///
    /// Drops broker-assigned fields (sequence number, enqueued time, delivery
    /// count, ...) and the dead-letter reason properties.
    pub fn to_sendable(&self) -> ServiceBusMessage {
        ServiceBusMessage {
            body: self.raw_body.clone(),
            content_type: self.broker_properties.content_type.clone(),
            message_id: self.broker_properties.message_id.clone(),
            correlation_id: self.broker_properties.correlation_id.clone(),
            session_id: self.broker_properties.session_id.clone(),
            label: self.broker_properties.label.clone(),
            to: self.broker_properties.to.clone(),
            reply_to: self.broker_properties.reply_to.clone(),
            // PartitionKey is deliberately dropped: it must match SessionId on
            // session entities and is meaningless on unpartitioned targets.
            custom_properties: self
                .raw_custom_properties
                .iter()
                .filter(|(k, _)| {
                    !DEAD_LETTER_PROPERTIES
                        .iter()
                        .any(|dl| k.eq_ignore_ascii_case(dl))
                })
                .cloned()
                .collect(),
            ..Default::default()
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
mod message_tests {
    use super::*;

    fn received(props: &[(&str, &str)]) -> ReceivedMessage {
        ReceivedMessage::new(
            b"{}".to_vec(),
            BrokerProperties::default(),
            props
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    }

    #[test]
    fn display_values_unquote_strings_but_keep_numbers() {
        assert_eq!(property_display_value("\"abc\""), "abc");
        assert_eq!(property_display_value("\"say \\\"hi\\\"\""), "say \"hi\"");
        assert_eq!(property_display_value("42"), "42");
        assert_eq!(property_display_value("true"), "true");
    }

    #[test]
    fn user_text_becomes_an_escaped_string_literal() {
        assert_eq!(property_literal_from_text("abc"), "\"abc\"");
        assert_eq!(property_literal_from_text("a\"b"), "\"a\\\"b\"");
    }

    #[test]
    fn resend_copy_keeps_property_types_and_drops_dead_letter_reasons() {
        let msg = received(&[
            ("count", "42"),
            ("tenant", "\"acme\""),
            ("DeadLetterReason", "\"MaxDeliveryCountExceeded\""),
            ("DeadLetterErrorDescription", "\"boom\""),
        ]);
        let copy = msg.to_sendable();
        assert_eq!(
            copy.custom_properties,
            vec![
                ("count".to_string(), "42".to_string()),
                ("tenant".to_string(), "\"acme\"".to_string()),
            ]
        );
    }

    #[test]
    fn binary_bodies_are_flagged_and_copied_byte_for_byte() {
        let msg = ReceivedMessage::new(vec![0xff, 0xfe, 0x00], BrokerProperties::default(), vec![]);
        assert!(!msg.body_is_text());
        assert_eq!(msg.to_sendable().body, vec![0xff, 0xfe, 0x00]);
    }
}
