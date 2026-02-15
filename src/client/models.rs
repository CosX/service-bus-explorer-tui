use serde::{Deserialize, Serialize};

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
pub struct RuleDescription {
    pub name: String,
    pub filter: RuleFilter,
    pub action: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub enum RuleFilter {
    #[default]
    TrueFilter,
    SqlFilter {
        expression: String,
    },
    CorrelationFilter {
        correlation_id: Option<String>,
        message_id: Option<String>,
        to: Option<String>,
        reply_to: Option<String>,
        label: Option<String>,
        session_id: Option<String>,
        content_type: Option<String>,
        properties: Vec<(String, String)>,
    },
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
