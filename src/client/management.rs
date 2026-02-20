use reqwest::Client;
use serde::Deserialize;

use super::auth::ConnectionConfig;
use super::error::{Result, ServiceBusError};
use super::models::*;

/// Client for Azure Service Bus management-plane operations (ATOM XML feeds).
#[derive(Clone)]
pub struct ManagementClient {
    config: ConnectionConfig,
    http: Client,
}

// ──────────────────────────── ATOM XML types ────────────────────────────
// These are internal deserialization structs for the ATOM feed responses.

#[derive(Debug, Deserialize)]
struct AtomFeed {
    #[serde(rename = "entry", default)]
    entries: Vec<AtomEntry>,
}

#[derive(Debug, Deserialize)]
struct AtomEntry {
    title: Option<AtomText>,
    content: Option<AtomContent>,
}

#[derive(Debug, Deserialize)]
struct AtomText {
    #[serde(rename = "$value", default)]
    value: String,
}

#[derive(Debug, Deserialize)]
struct AtomContent {
    #[serde(rename = "QueueDescription")]
    queue_description: Option<AtomQueueDescription>,
    #[serde(rename = "TopicDescription")]
    topic_description: Option<AtomTopicDescription>,
    #[serde(rename = "SubscriptionDescription")]
    subscription_description: Option<AtomSubscriptionDescription>,
    #[serde(rename = "RuleDescription")]
    rule_description: Option<AtomRuleDescription>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct AtomQueueDescription {
    #[serde(rename = "LockDuration")]
    lock_duration: Option<String>,
    #[serde(rename = "MaxSizeInMegabytes")]
    max_size_in_megabytes: Option<i64>,
    #[serde(rename = "RequiresDuplicateDetection")]
    requires_duplicate_detection: Option<bool>,
    #[serde(rename = "RequiresSession")]
    requires_session: Option<bool>,
    #[serde(rename = "DefaultMessageTimeToLive")]
    default_message_time_to_live: Option<String>,
    #[serde(rename = "DeadLetteringOnMessageExpiration")]
    dead_lettering_on_message_expiration: Option<bool>,
    #[serde(rename = "DuplicateDetectionHistoryTimeWindow")]
    duplicate_detection_history_time_window: Option<String>,
    #[serde(rename = "MaxDeliveryCount")]
    max_delivery_count: Option<i32>,
    #[serde(rename = "EnableBatchedOperations")]
    enable_batched_operations: Option<bool>,
    #[serde(rename = "Status")]
    status: Option<String>,
    #[serde(rename = "ForwardTo")]
    forward_to: Option<String>,
    #[serde(rename = "ForwardDeadLetteredMessagesTo")]
    forward_dead_lettered_messages_to: Option<String>,
    #[serde(rename = "AutoDeleteOnIdle")]
    auto_delete_on_idle: Option<String>,
    #[serde(rename = "EnablePartitioning")]
    enable_partitioning: Option<bool>,
    #[serde(rename = "UserMetadata")]
    user_metadata: Option<String>,
    // Runtime info fields (returned in same description)
    #[serde(rename = "MessageCount")]
    message_count: Option<i64>,
    #[serde(rename = "SizeInBytes")]
    size_in_bytes: Option<i64>,
    #[serde(rename = "CreatedAt")]
    created_at: Option<String>,
    #[serde(rename = "UpdatedAt")]
    updated_at: Option<String>,
    #[serde(rename = "AccessedAt")]
    accessed_at: Option<String>,
    #[serde(rename = "CountDetails")]
    count_details: Option<AtomCountDetails>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct AtomTopicDescription {
    #[serde(rename = "MaxSizeInMegabytes")]
    max_size_in_megabytes: Option<i64>,
    #[serde(rename = "DefaultMessageTimeToLive")]
    default_message_time_to_live: Option<String>,
    #[serde(rename = "RequiresDuplicateDetection")]
    requires_duplicate_detection: Option<bool>,
    #[serde(rename = "DuplicateDetectionHistoryTimeWindow")]
    duplicate_detection_history_time_window: Option<String>,
    #[serde(rename = "EnableBatchedOperations")]
    enable_batched_operations: Option<bool>,
    #[serde(rename = "Status")]
    status: Option<String>,
    #[serde(rename = "SupportOrdering")]
    support_ordering: Option<bool>,
    #[serde(rename = "AutoDeleteOnIdle")]
    auto_delete_on_idle: Option<String>,
    #[serde(rename = "EnablePartitioning")]
    enable_partitioning: Option<bool>,
    #[serde(rename = "UserMetadata")]
    user_metadata: Option<String>,
    #[serde(rename = "SubscriptionCount")]
    subscription_count: Option<i64>,
    #[serde(rename = "SizeInBytes")]
    size_in_bytes: Option<i64>,
    #[serde(rename = "CreatedAt")]
    created_at: Option<String>,
    #[serde(rename = "UpdatedAt")]
    updated_at: Option<String>,
    #[serde(rename = "AccessedAt")]
    accessed_at: Option<String>,
    #[serde(rename = "CountDetails")]
    count_details: Option<AtomCountDetails>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct AtomSubscriptionDescription {
    #[serde(rename = "LockDuration")]
    lock_duration: Option<String>,
    #[serde(rename = "RequiresSession")]
    requires_session: Option<bool>,
    #[serde(rename = "DefaultMessageTimeToLive")]
    default_message_time_to_live: Option<String>,
    #[serde(rename = "DeadLetteringOnMessageExpiration")]
    dead_lettering_on_message_expiration: Option<bool>,
    #[serde(rename = "DeadLetteringOnFilterEvaluationExceptions")]
    dead_lettering_on_filter_evaluation_exceptions: Option<bool>,
    #[serde(rename = "MaxDeliveryCount")]
    max_delivery_count: Option<i32>,
    #[serde(rename = "EnableBatchedOperations")]
    enable_batched_operations: Option<bool>,
    #[serde(rename = "Status")]
    status: Option<String>,
    #[serde(rename = "ForwardTo")]
    forward_to: Option<String>,
    #[serde(rename = "ForwardDeadLetteredMessagesTo")]
    forward_dead_lettered_messages_to: Option<String>,
    #[serde(rename = "AutoDeleteOnIdle")]
    auto_delete_on_idle: Option<String>,
    #[serde(rename = "UserMetadata")]
    user_metadata: Option<String>,
    #[serde(rename = "MessageCount")]
    message_count: Option<i64>,
    #[serde(rename = "CreatedAt")]
    created_at: Option<String>,
    #[serde(rename = "UpdatedAt")]
    updated_at: Option<String>,
    #[serde(rename = "AccessedAt")]
    accessed_at: Option<String>,
    #[serde(rename = "CountDetails")]
    count_details: Option<AtomCountDetails>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct AtomCountDetails {
    #[serde(rename = "ActiveMessageCount")]
    active_message_count: Option<i64>,
    #[serde(rename = "DeadLetterMessageCount")]
    dead_letter_message_count: Option<i64>,
    #[serde(rename = "ScheduledMessageCount")]
    scheduled_message_count: Option<i64>,
    #[serde(rename = "TransferMessageCount")]
    transfer_message_count: Option<i64>,
    #[serde(rename = "TransferDeadLetterMessageCount")]
    transfer_dead_letter_message_count: Option<i64>,
}

#[derive(Debug, Deserialize, Default)]
struct AtomRuleDescription {
    #[serde(rename = "Filter")]
    filter: Option<AtomRuleFilter>,
    #[serde(rename = "Action")]
    action: Option<AtomRuleAction>,
    #[serde(rename = "Name")]
    name: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct AtomRuleFilter {
    #[serde(rename = "SqlExpression")]
    sql_expression: Option<String>,
    #[serde(rename = "CorrelationId")]
    correlation_id: Option<String>,
    // Type hint from the xsi:type attribute
    #[serde(rename = "@type", default)]
    filter_type: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct AtomRuleAction {
    #[serde(rename = "SqlExpression")]
    sql_expression: Option<String>,
}

// ──────────────────────────── ATOM XML building ────────────────────────────

fn wrap_atom_entry(inner_xml: &str) -> String {
    format!(
        r#"<entry xmlns="http://www.w3.org/2005/Atom">
  <content type="application/xml">
    {}
  </content>
</entry>"#,
        inner_xml
    )
}

fn queue_description_xml(desc: &QueueDescription) -> String {
    let mut xml = String::from(
        r#"<QueueDescription xmlns="http://schemas.microsoft.com/netservices/2010/10/servicebus/connect" xmlns:i="http://www.w3.org/2001/XMLSchema-instance">"#,
    );
    if let Some(ref v) = desc.lock_duration {
        xml.push_str(&format!("<LockDuration>{}</LockDuration>", v));
    }
    if let Some(v) = desc.max_size_in_megabytes {
        xml.push_str(&format!("<MaxSizeInMegabytes>{}</MaxSizeInMegabytes>", v));
    }
    if let Some(v) = desc.requires_duplicate_detection {
        xml.push_str(&format!(
            "<RequiresDuplicateDetection>{}</RequiresDuplicateDetection>",
            v
        ));
    }
    if let Some(v) = desc.requires_session {
        xml.push_str(&format!("<RequiresSession>{}</RequiresSession>", v));
    }
    if let Some(ref v) = desc.default_message_time_to_live {
        xml.push_str(&format!(
            "<DefaultMessageTimeToLive>{}</DefaultMessageTimeToLive>",
            v
        ));
    }
    if let Some(v) = desc.dead_lettering_on_message_expiration {
        xml.push_str(&format!(
            "<DeadLetteringOnMessageExpiration>{}</DeadLetteringOnMessageExpiration>",
            v
        ));
    }
    if let Some(ref v) = desc.duplicate_detection_history_time_window {
        xml.push_str(&format!(
            "<DuplicateDetectionHistoryTimeWindow>{}</DuplicateDetectionHistoryTimeWindow>",
            v
        ));
    }
    if let Some(v) = desc.max_delivery_count {
        xml.push_str(&format!("<MaxDeliveryCount>{}</MaxDeliveryCount>", v));
    }
    if let Some(v) = desc.enable_batched_operations {
        xml.push_str(&format!(
            "<EnableBatchedOperations>{}</EnableBatchedOperations>",
            v
        ));
    }
    if let Some(ref v) = desc.status {
        xml.push_str(&format!("<Status>{}</Status>", v));
    }
    if let Some(ref v) = desc.forward_to {
        xml.push_str(&format!("<ForwardTo>{}</ForwardTo>", v));
    }
    if let Some(ref v) = desc.forward_dead_lettered_messages_to {
        xml.push_str(&format!(
            "<ForwardDeadLetteredMessagesTo>{}</ForwardDeadLetteredMessagesTo>",
            v
        ));
    }
    if let Some(ref v) = desc.auto_delete_on_idle {
        xml.push_str(&format!("<AutoDeleteOnIdle>{}</AutoDeleteOnIdle>", v));
    }
    if let Some(v) = desc.enable_partitioning {
        xml.push_str(&format!("<EnablePartitioning>{}</EnablePartitioning>", v));
    }
    xml.push_str("</QueueDescription>");
    xml
}

fn topic_description_xml(desc: &TopicDescription) -> String {
    let mut xml = String::from(
        r#"<TopicDescription xmlns="http://schemas.microsoft.com/netservices/2010/10/servicebus/connect" xmlns:i="http://www.w3.org/2001/XMLSchema-instance">"#,
    );
    if let Some(v) = desc.max_size_in_megabytes {
        xml.push_str(&format!("<MaxSizeInMegabytes>{}</MaxSizeInMegabytes>", v));
    }
    if let Some(ref v) = desc.default_message_time_to_live {
        xml.push_str(&format!(
            "<DefaultMessageTimeToLive>{}</DefaultMessageTimeToLive>",
            v
        ));
    }
    if let Some(v) = desc.requires_duplicate_detection {
        xml.push_str(&format!(
            "<RequiresDuplicateDetection>{}</RequiresDuplicateDetection>",
            v
        ));
    }
    if let Some(v) = desc.enable_batched_operations {
        xml.push_str(&format!(
            "<EnableBatchedOperations>{}</EnableBatchedOperations>",
            v
        ));
    }
    if let Some(ref v) = desc.status {
        xml.push_str(&format!("<Status>{}</Status>", v));
    }
    if let Some(v) = desc.support_ordering {
        xml.push_str(&format!("<SupportOrdering>{}</SupportOrdering>", v));
    }
    if let Some(ref v) = desc.auto_delete_on_idle {
        xml.push_str(&format!("<AutoDeleteOnIdle>{}</AutoDeleteOnIdle>", v));
    }
    if let Some(v) = desc.enable_partitioning {
        xml.push_str(&format!("<EnablePartitioning>{}</EnablePartitioning>", v));
    }
    xml.push_str("</TopicDescription>");
    xml
}

fn subscription_description_xml(desc: &SubscriptionDescription) -> String {
    let mut xml = String::from(
        r#"<SubscriptionDescription xmlns="http://schemas.microsoft.com/netservices/2010/10/servicebus/connect" xmlns:i="http://www.w3.org/2001/XMLSchema-instance">"#,
    );
    if let Some(ref v) = desc.lock_duration {
        xml.push_str(&format!("<LockDuration>{}</LockDuration>", v));
    }
    if let Some(v) = desc.requires_session {
        xml.push_str(&format!("<RequiresSession>{}</RequiresSession>", v));
    }
    if let Some(ref v) = desc.default_message_time_to_live {
        xml.push_str(&format!(
            "<DefaultMessageTimeToLive>{}</DefaultMessageTimeToLive>",
            v
        ));
    }
    if let Some(v) = desc.dead_lettering_on_message_expiration {
        xml.push_str(&format!(
            "<DeadLetteringOnMessageExpiration>{}</DeadLetteringOnMessageExpiration>",
            v
        ));
    }
    if let Some(v) = desc.max_delivery_count {
        xml.push_str(&format!("<MaxDeliveryCount>{}</MaxDeliveryCount>", v));
    }
    if let Some(v) = desc.enable_batched_operations {
        xml.push_str(&format!(
            "<EnableBatchedOperations>{}</EnableBatchedOperations>",
            v
        ));
    }
    if let Some(ref v) = desc.status {
        xml.push_str(&format!("<Status>{}</Status>", v));
    }
    if let Some(ref v) = desc.forward_to {
        xml.push_str(&format!("<ForwardTo>{}</ForwardTo>", v));
    }
    if let Some(ref v) = desc.forward_dead_lettered_messages_to {
        xml.push_str(&format!(
            "<ForwardDeadLetteredMessagesTo>{}</ForwardDeadLetteredMessagesTo>",
            v
        ));
    }
    if let Some(ref v) = desc.auto_delete_on_idle {
        xml.push_str(&format!("<AutoDeleteOnIdle>{}</AutoDeleteOnIdle>", v));
    }
    xml.push_str("</SubscriptionDescription>");
    xml
}

// ──────────────────────────── Implementation ────────────────────────────

impl ManagementClient {
    pub fn new(config: ConnectionConfig) -> Self {
        Self {
            config,
            http: Client::new(),
        }
    }

    async fn get_atom(&self, path: &str) -> Result<String> {
        let url = format!("{}/{}?api-version=2017-04", self.config.endpoint, path);
        let token = self.config.namespace_token().await?;

        let resp = self
            .http
            .get(&url)
            .header("Authorization", token)
            .header("Content-Type", "application/atom+xml;charset=utf-8")
            .send()
            .await?;

        let status = resp.status().as_u16();
        let body = resp.text().await?;

        if status == 404 {
            return Err(ServiceBusError::NotFound(path.to_string()));
        }
        if status >= 400 {
            return Err(ServiceBusError::Api { status, body });
        }

        Ok(body)
    }

    async fn put_atom(&self, path: &str, body: &str) -> Result<String> {
        let url = format!("{}/{}?api-version=2017-04", self.config.endpoint, path);
        let token = self.config.namespace_token().await?;

        let resp = self
            .http
            .put(&url)
            .header("Authorization", token)
            .header("Content-Type", "application/atom+xml;charset=utf-8")
            .body(body.to_string())
            .send()
            .await?;

        let status = resp.status().as_u16();
        let resp_body = resp.text().await?;

        if status >= 400 {
            return Err(ServiceBusError::Api {
                status,
                body: resp_body,
            });
        }
        Ok(resp_body)
    }

    async fn delete_entity(&self, path: &str) -> Result<()> {
        let url = format!("{}/{}?api-version=2017-04", self.config.endpoint, path);
        let token = self.config.namespace_token().await?;

        let resp = self
            .http
            .delete(&url)
            .header("Authorization", token)
            .send()
            .await?;

        let status = resp.status().as_u16();
        if status == 404 {
            return Err(ServiceBusError::NotFound(path.to_string()));
        }
        if status >= 400 {
            let body = resp.text().await?;
            return Err(ServiceBusError::Api { status, body });
        }
        Ok(())
    }

    // ────────── Queues ──────────

    pub async fn list_queues(&self) -> Result<Vec<QueueDescription>> {
        let xml = self.get_atom("$Resources/Queues").await?;
        parse_queue_feed(&xml)
    }

    /// List queues with (active_message_count, dead_letter_message_count) from the same feed.
    pub async fn list_queues_with_counts(&self) -> Result<Vec<(QueueDescription, i64, i64)>> {
        let xml = self.get_atom("$Resources/Queues").await?;
        parse_queue_feed_with_counts(&xml)
    }

    pub async fn get_queue(&self, name: &str) -> Result<QueueDescription> {
        let xml = self.get_atom(name).await?;
        parse_single_queue(&xml)
    }

    pub async fn get_queue_runtime_info(&self, name: &str) -> Result<QueueRuntimeInfo> {
        let xml = self.get_atom(name).await?;
        parse_queue_runtime_info(name, &xml)
    }

    pub async fn create_queue(&self, desc: &QueueDescription) -> Result<QueueDescription> {
        let inner = queue_description_xml(desc);
        let body = wrap_atom_entry(&inner);
        let xml = self.put_atom(&desc.name, &body).await?;
        parse_single_queue(&xml)
    }

    pub async fn update_queue(&self, desc: &QueueDescription) -> Result<QueueDescription> {
        let inner = queue_description_xml(desc);
        let body = wrap_atom_entry(&inner);
        let xml = self.put_atom(&desc.name, &body).await?;
        parse_single_queue(&xml)
    }

    pub async fn delete_queue(&self, name: &str) -> Result<()> {
        self.delete_entity(name).await
    }

    // ────────── Topics ──────────

    pub async fn list_topics(&self) -> Result<Vec<TopicDescription>> {
        let xml = self.get_atom("$Resources/Topics").await?;
        parse_topic_feed(&xml)
    }

    pub async fn get_topic(&self, name: &str) -> Result<TopicDescription> {
        let xml = self.get_atom(name).await?;
        parse_single_topic(&xml)
    }

    pub async fn get_topic_runtime_info(&self, name: &str) -> Result<TopicRuntimeInfo> {
        let xml = self.get_atom(name).await?;
        parse_topic_runtime_info(name, &xml)
    }

    pub async fn create_topic(&self, desc: &TopicDescription) -> Result<TopicDescription> {
        let inner = topic_description_xml(desc);
        let body = wrap_atom_entry(&inner);
        let xml = self.put_atom(&desc.name, &body).await?;
        parse_single_topic(&xml)
    }

    pub async fn update_topic(&self, desc: &TopicDescription) -> Result<TopicDescription> {
        let inner = topic_description_xml(desc);
        let body = wrap_atom_entry(&inner);
        let xml = self.put_atom(&desc.name, &body).await?;
        parse_single_topic(&xml)
    }

    pub async fn delete_topic(&self, name: &str) -> Result<()> {
        self.delete_entity(name).await
    }

    // ────────── Subscriptions ──────────

    pub async fn list_subscriptions(
        &self,
        topic_name: &str,
    ) -> Result<Vec<SubscriptionDescription>> {
        let xml = self
            .get_atom(&format!("{}/Subscriptions", topic_name))
            .await?;
        parse_subscription_feed(topic_name, &xml)
    }

    /// List subscriptions with (active_message_count, dead_letter_message_count) from the same feed.
    pub async fn list_subscriptions_with_counts(
        &self,
        topic_name: &str,
    ) -> Result<Vec<(SubscriptionDescription, i64, i64)>> {
        let xml = self
            .get_atom(&format!("{}/Subscriptions", topic_name))
            .await?;
        parse_subscription_feed_with_counts(topic_name, &xml)
    }

    pub async fn get_subscription(
        &self,
        topic_name: &str,
        sub_name: &str,
    ) -> Result<SubscriptionDescription> {
        let xml = self
            .get_atom(&format!("{}/Subscriptions/{}", topic_name, sub_name))
            .await?;
        parse_single_subscription(topic_name, sub_name, &xml)
    }

    pub async fn get_subscription_runtime_info(
        &self,
        topic_name: &str,
        sub_name: &str,
    ) -> Result<SubscriptionRuntimeInfo> {
        let xml = self
            .get_atom(&format!("{}/Subscriptions/{}", topic_name, sub_name))
            .await?;
        parse_subscription_runtime_info(topic_name, sub_name, &xml)
    }

    pub async fn create_subscription(
        &self,
        desc: &SubscriptionDescription,
    ) -> Result<SubscriptionDescription> {
        let inner = subscription_description_xml(desc);
        let body = wrap_atom_entry(&inner);
        let path = format!("{}/Subscriptions/{}", desc.topic_name, desc.name);
        let xml = self.put_atom(&path, &body).await?;
        parse_single_subscription(&desc.topic_name, &desc.name, &xml)
    }

    pub async fn update_subscription(
        &self,
        desc: &SubscriptionDescription,
    ) -> Result<SubscriptionDescription> {
        let inner = subscription_description_xml(desc);
        let body = wrap_atom_entry(&inner);
        let path = format!("{}/Subscriptions/{}", desc.topic_name, desc.name);
        let xml = self.put_atom(&path, &body).await?;
        parse_single_subscription(&desc.topic_name, &desc.name, &xml)
    }

    pub async fn delete_subscription(&self, topic_name: &str, sub_name: &str) -> Result<()> {
        self.delete_entity(&format!("{}/Subscriptions/{}", topic_name, sub_name))
            .await
    }

    // ────────── Rules ──────────

    pub async fn list_rules(
        &self,
        topic_name: &str,
        sub_name: &str,
    ) -> Result<Vec<RuleDescription>> {
        let xml = self
            .get_atom(&format!("{}/Subscriptions/{}/Rules", topic_name, sub_name))
            .await?;
        parse_rule_feed(&xml)
    }

    pub async fn delete_rule(
        &self,
        topic_name: &str,
        sub_name: &str,
        rule_name: &str,
    ) -> Result<()> {
        self.delete_entity(&format!(
            "{}/Subscriptions/{}/Rules/{}",
            topic_name, sub_name, rule_name
        ))
        .await
    }
}

// ──────────────────────────── XML Parsing helpers ────────────────────────────
// The ATOM feed XML is complex. We do best-effort extraction using quick-xml.
// For resilience, we parse the raw XML by searching for specific elements
// rather than relying on strict schema compliance.

fn extract_entries(xml: &str) -> Vec<String> {
    let mut entries = Vec::new();
    let mut remaining = xml;
    while let Some(start) = remaining.find("<entry") {
        // Find matching </entry> — handle nested entry unlikely but be safe
        if let Some(end) = remaining[start..].find("</entry>") {
            let entry_end = start + end + "</entry>".len();
            entries.push(remaining[start..entry_end].to_string());
            remaining = &remaining[entry_end..];
        } else {
            break;
        }
    }
    entries
}

fn extract_element(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{}", tag);
    let close = format!("</{}>", tag);
    if let Some(start_pos) = xml.find(&open) {
        // Find the end of the opening tag
        if let Some(gt_pos) = xml[start_pos..].find('>') {
            let content_start = start_pos + gt_pos + 1;
            if let Some(end_pos) = xml[content_start..].find(&close) {
                return Some(xml[content_start..content_start + end_pos].to_string());
            }
        }
    }
    None
}

fn extract_element_value(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    if let Some(start_pos) = xml.find(&open) {
        let content_start = start_pos + open.len();
        if let Some(end_pos) = xml[content_start..].find(&close) {
            let val = xml[content_start..content_start + end_pos]
                .trim()
                .to_string();
            if val.is_empty() {
                return None;
            }
            return Some(val);
        }
    }
    None
}

fn extract_title(entry_xml: &str) -> String {
    // Azure ATOM feeds use <title type="text">name</title>, so we must use
    // extract_element (handles attributes) rather than extract_element_value
    // (which requires an exact <title> open tag with no attributes).
    extract_element(entry_xml, "title")
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn parse_optional_i64(xml: &str, tag: &str) -> Option<i64> {
    extract_element_value(xml, tag).and_then(|v| v.parse().ok())
}

fn parse_optional_i32(xml: &str, tag: &str) -> Option<i32> {
    extract_element_value(xml, tag).and_then(|v| v.parse().ok())
}

fn parse_optional_bool(xml: &str, tag: &str) -> Option<bool> {
    extract_element_value(xml, tag).and_then(|v| v.parse().ok())
}

/// Extract an element's text value by local name, ignoring any XML namespace prefix.
///
/// Azure's WCF serializer assigns auto-generated namespace prefixes (`d2p1:`, `d3p1:`, etc.)
/// that vary depending on element nesting depth. This function finds the element regardless
/// of which prefix is used, falling back to an unprefixed match.
fn extract_value_any_ns(xml: &str, local_name: &str) -> Option<String> {
    // Try unprefixed first (cheapest check)
    if let Some(v) = extract_element_value(xml, local_name) {
        return Some(v);
    }
    // Search for ":LocalName>" — any namespace prefix
    let suffix = format!(":{}>", local_name);
    if let Some(suffix_pos) = xml.find(&suffix) {
        // Walk backward to find the '<' that opens this tag
        let before = &xml[..suffix_pos];
        if let Some(lt_pos) = before.rfind('<') {
            let full_tag = &xml[lt_pos + 1..suffix_pos + suffix.len() - 1];
            return extract_element_value(xml, full_tag);
        }
    }
    None
}

fn parse_count_details(xml: &str) -> (i64, i64, i64, i64, i64) {
    let cd = extract_element(xml, "CountDetails").unwrap_or_default();
    let active = extract_value_any_ns(&cd, "ActiveMessageCount")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let dlq = extract_value_any_ns(&cd, "DeadLetterMessageCount")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let scheduled = extract_value_any_ns(&cd, "ScheduledMessageCount")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let transfer = extract_value_any_ns(&cd, "TransferMessageCount")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let transfer_dlq = extract_value_any_ns(&cd, "TransferDeadLetterMessageCount")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    (active, dlq, scheduled, transfer, transfer_dlq)
}

fn parse_queue_from_entry(entry_xml: &str) -> QueueDescription {
    let name = extract_title(entry_xml);
    QueueDescription {
        name,
        lock_duration: extract_element_value(entry_xml, "LockDuration"),
        max_size_in_megabytes: parse_optional_i64(entry_xml, "MaxSizeInMegabytes"),
        requires_duplicate_detection: parse_optional_bool(entry_xml, "RequiresDuplicateDetection"),
        requires_session: parse_optional_bool(entry_xml, "RequiresSession"),
        default_message_time_to_live: extract_element_value(entry_xml, "DefaultMessageTimeToLive"),
        dead_lettering_on_message_expiration: parse_optional_bool(
            entry_xml,
            "DeadLetteringOnMessageExpiration",
        ),
        duplicate_detection_history_time_window: extract_element_value(
            entry_xml,
            "DuplicateDetectionHistoryTimeWindow",
        ),
        max_delivery_count: parse_optional_i32(entry_xml, "MaxDeliveryCount"),
        enable_batched_operations: parse_optional_bool(entry_xml, "EnableBatchedOperations"),
        status: extract_element_value(entry_xml, "Status"),
        forward_to: extract_element_value(entry_xml, "ForwardTo"),
        forward_dead_lettered_messages_to: extract_element_value(
            entry_xml,
            "ForwardDeadLetteredMessagesTo",
        ),
        auto_delete_on_idle: extract_element_value(entry_xml, "AutoDeleteOnIdle"),
        enable_partitioning: parse_optional_bool(entry_xml, "EnablePartitioning"),
        user_metadata: extract_element_value(entry_xml, "UserMetadata"),
    }
}

fn parse_queue_feed(xml: &str) -> Result<Vec<QueueDescription>> {
    Ok(extract_entries(xml)
        .into_iter()
        .map(|e| parse_queue_from_entry(&e))
        .collect())
}

fn parse_queue_feed_with_counts(xml: &str) -> Result<Vec<(QueueDescription, i64, i64)>> {
    Ok(extract_entries(xml)
        .into_iter()
        .map(|e| {
            let desc = parse_queue_from_entry(&e);
            let (active, dlq, _, _, _) = parse_count_details(&e);
            (desc, active, dlq)
        })
        .collect())
}

fn parse_single_queue(xml: &str) -> Result<QueueDescription> {
    Ok(parse_queue_from_entry(xml))
}

fn parse_queue_runtime_info(name: &str, xml: &str) -> Result<QueueRuntimeInfo> {
    let (active, dlq, scheduled, transfer, transfer_dlq) = parse_count_details(xml);
    Ok(QueueRuntimeInfo {
        name: name.to_string(),
        active_message_count: active,
        dead_letter_message_count: dlq,
        scheduled_message_count: scheduled,
        transfer_message_count: transfer,
        transfer_dead_letter_message_count: transfer_dlq,
        size_in_bytes: parse_optional_i64(xml, "SizeInBytes").unwrap_or(0),
        created_at: extract_element_value(xml, "CreatedAt"),
        updated_at: extract_element_value(xml, "UpdatedAt"),
        accessed_at: extract_element_value(xml, "AccessedAt"),
        message_count: parse_optional_i64(xml, "MessageCount").unwrap_or(0),
    })
}

fn parse_topic_from_entry(entry_xml: &str) -> TopicDescription {
    let name = extract_title(entry_xml);
    TopicDescription {
        name,
        max_size_in_megabytes: parse_optional_i64(entry_xml, "MaxSizeInMegabytes"),
        default_message_time_to_live: extract_element_value(entry_xml, "DefaultMessageTimeToLive"),
        requires_duplicate_detection: parse_optional_bool(entry_xml, "RequiresDuplicateDetection"),
        duplicate_detection_history_time_window: extract_element_value(
            entry_xml,
            "DuplicateDetectionHistoryTimeWindow",
        ),
        enable_batched_operations: parse_optional_bool(entry_xml, "EnableBatchedOperations"),
        status: extract_element_value(entry_xml, "Status"),
        support_ordering: parse_optional_bool(entry_xml, "SupportOrdering"),
        auto_delete_on_idle: extract_element_value(entry_xml, "AutoDeleteOnIdle"),
        enable_partitioning: parse_optional_bool(entry_xml, "EnablePartitioning"),
        user_metadata: extract_element_value(entry_xml, "UserMetadata"),
    }
}

fn parse_topic_feed(xml: &str) -> Result<Vec<TopicDescription>> {
    Ok(extract_entries(xml)
        .into_iter()
        .map(|e| parse_topic_from_entry(&e))
        .collect())
}

fn parse_single_topic(xml: &str) -> Result<TopicDescription> {
    Ok(parse_topic_from_entry(xml))
}

fn parse_topic_runtime_info(name: &str, xml: &str) -> Result<TopicRuntimeInfo> {
    let (_, _, scheduled, _, _) = parse_count_details(xml);
    Ok(TopicRuntimeInfo {
        name: name.to_string(),
        subscription_count: parse_optional_i64(xml, "SubscriptionCount").unwrap_or(0),
        size_in_bytes: parse_optional_i64(xml, "SizeInBytes").unwrap_or(0),
        created_at: extract_element_value(xml, "CreatedAt"),
        updated_at: extract_element_value(xml, "UpdatedAt"),
        accessed_at: extract_element_value(xml, "AccessedAt"),
        scheduled_message_count: scheduled,
        active_message_count: 0,
        dead_letter_message_count: 0,
    })
}

fn parse_subscription_from_entry(topic_name: &str, entry_xml: &str) -> SubscriptionDescription {
    let name = extract_title(entry_xml);
    SubscriptionDescription {
        name,
        topic_name: topic_name.to_string(),
        lock_duration: extract_element_value(entry_xml, "LockDuration"),
        requires_session: parse_optional_bool(entry_xml, "RequiresSession"),
        default_message_time_to_live: extract_element_value(entry_xml, "DefaultMessageTimeToLive"),
        dead_lettering_on_message_expiration: parse_optional_bool(
            entry_xml,
            "DeadLetteringOnMessageExpiration",
        ),
        dead_lettering_on_filter_evaluation_exceptions: parse_optional_bool(
            entry_xml,
            "DeadLetteringOnFilterEvaluationExceptions",
        ),
        max_delivery_count: parse_optional_i32(entry_xml, "MaxDeliveryCount"),
        enable_batched_operations: parse_optional_bool(entry_xml, "EnableBatchedOperations"),
        status: extract_element_value(entry_xml, "Status"),
        forward_to: extract_element_value(entry_xml, "ForwardTo"),
        forward_dead_lettered_messages_to: extract_element_value(
            entry_xml,
            "ForwardDeadLetteredMessagesTo",
        ),
        auto_delete_on_idle: extract_element_value(entry_xml, "AutoDeleteOnIdle"),
        user_metadata: extract_element_value(entry_xml, "UserMetadata"),
    }
}

fn parse_subscription_feed(topic_name: &str, xml: &str) -> Result<Vec<SubscriptionDescription>> {
    Ok(extract_entries(xml)
        .into_iter()
        .map(|e| parse_subscription_from_entry(topic_name, &e))
        .collect())
}

fn parse_subscription_feed_with_counts(
    topic_name: &str,
    xml: &str,
) -> Result<Vec<(SubscriptionDescription, i64, i64)>> {
    Ok(extract_entries(xml)
        .into_iter()
        .map(|e| {
            let desc = parse_subscription_from_entry(topic_name, &e);
            let (active, dlq, _, _, _) = parse_count_details(&e);
            (desc, active, dlq)
        })
        .collect())
}

fn parse_single_subscription(
    topic_name: &str,
    sub_name: &str,
    xml: &str,
) -> Result<SubscriptionDescription> {
    let mut desc = parse_subscription_from_entry(topic_name, xml);
    if desc.name.is_empty() {
        desc.name = sub_name.to_string();
    }
    Ok(desc)
}

fn parse_subscription_runtime_info(
    topic_name: &str,
    sub_name: &str,
    xml: &str,
) -> Result<SubscriptionRuntimeInfo> {
    let (active, dlq, _, transfer, transfer_dlq) = parse_count_details(xml);
    Ok(SubscriptionRuntimeInfo {
        name: sub_name.to_string(),
        topic_name: topic_name.to_string(),
        active_message_count: active,
        dead_letter_message_count: dlq,
        transfer_message_count: transfer,
        transfer_dead_letter_message_count: transfer_dlq,
        message_count: parse_optional_i64(xml, "MessageCount").unwrap_or(0),
        created_at: extract_element_value(xml, "CreatedAt"),
        updated_at: extract_element_value(xml, "UpdatedAt"),
        accessed_at: extract_element_value(xml, "AccessedAt"),
    })
}

fn parse_rule_feed(xml: &str) -> Result<Vec<RuleDescription>> {
    Ok(extract_entries(xml)
        .into_iter()
        .map(|e| {
            let name = extract_title(&e);
            let filter = if let Some(sql) = extract_element_value(&e, "SqlExpression") {
                if sql == "1=1" {
                    RuleFilter::TrueFilter
                } else {
                    RuleFilter::SqlFilter { expression: sql }
                }
            } else {
                RuleFilter::TrueFilter
            };
            RuleDescription {
                name,
                filter,
                action: extract_element(&e, "Action")
                    .and_then(|a| extract_element_value(&a, "SqlExpression")),
            }
        })
        .collect())
}
