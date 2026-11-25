use std::borrow::Cow;
use std::time::Duration;

use reqwest::Client;

use super::auth::ConnectionConfig;
use super::error::{Result, ServiceBusError};
use super::models::*;

/// Client for Azure Service Bus management-plane operations (ATOM XML feeds).
#[derive(Clone)]
pub struct ManagementClient {
    config: ConnectionConfig,
    http: Client,
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
        let v = esc(v);
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
        let v = esc(v);
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
        let v = esc(v);
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
        let v = esc(v);
        xml.push_str(&format!("<Status>{}</Status>", v));
    }
    if let Some(ref v) = desc.forward_to {
        let v = esc(v);
        xml.push_str(&format!("<ForwardTo>{}</ForwardTo>", v));
    }
    if let Some(ref v) = desc.forward_dead_lettered_messages_to {
        let v = esc(v);
        xml.push_str(&format!(
            "<ForwardDeadLetteredMessagesTo>{}</ForwardDeadLetteredMessagesTo>",
            v
        ));
    }
    if let Some(ref v) = desc.auto_delete_on_idle {
        let v = esc(v);
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
        let v = esc(v);
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
        let v = esc(v);
        xml.push_str(&format!("<Status>{}</Status>", v));
    }
    if let Some(v) = desc.support_ordering {
        xml.push_str(&format!("<SupportOrdering>{}</SupportOrdering>", v));
    }
    if let Some(ref v) = desc.auto_delete_on_idle {
        let v = esc(v);
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
        let v = esc(v);
        xml.push_str(&format!("<LockDuration>{}</LockDuration>", v));
    }
    if let Some(v) = desc.requires_session {
        xml.push_str(&format!("<RequiresSession>{}</RequiresSession>", v));
    }
    if let Some(ref v) = desc.default_message_time_to_live {
        let v = esc(v);
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
        let v = esc(v);
        xml.push_str(&format!("<Status>{}</Status>", v));
    }
    if let Some(ref v) = desc.forward_to {
        let v = esc(v);
        xml.push_str(&format!("<ForwardTo>{}</ForwardTo>", v));
    }
    if let Some(ref v) = desc.forward_dead_lettered_messages_to {
        let v = esc(v);
        xml.push_str(&format!(
            "<ForwardDeadLetteredMessagesTo>{}</ForwardDeadLetteredMessagesTo>",
            v
        ));
    }
    if let Some(ref v) = desc.auto_delete_on_idle {
        let v = esc(v);
        xml.push_str(&format!("<AutoDeleteOnIdle>{}</AutoDeleteOnIdle>", v));
    }
    xml.push_str("</SubscriptionDescription>");
    xml
}

/// Per-request timeout for management calls.
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

/// Entries per ATOM feed page. Service Bus caps feeds at 100 per request.
const FEED_PAGE_SIZE: usize = 100;

/// Upper bound on feed pages (1M entities), so a server bug cannot loop forever.
const MAX_FEED_PAGES: usize = 10_000;

/// Escape a value for use as XML element text.
fn esc(value: &str) -> Cow<'_, str> {
    quick_xml::escape::escape(value)
}

/// Decode XML entities in element text (`&gt;` -> `>`). Falls back to the
/// raw text if it contains an entity quick-xml does not know.
fn unesc(value: &str) -> String {
    quick_xml::escape::unescape(value)
        .map(Cow::into_owned)
        .unwrap_or_else(|_| value.to_string())
}

fn to_cdata_safe(value: &str) -> String {
    value.replace("]]>", "]]]]><![CDATA[>")
}

fn subscription_rule_sql_xml(sql_expression: &str) -> String {
    let expr = to_cdata_safe(sql_expression);
    format!(
        r#"<RuleDescription xmlns="http://schemas.microsoft.com/netservices/2010/10/servicebus/connect" xmlns:i="http://www.w3.org/2001/XMLSchema-instance"><Filter i:type="SqlFilter"><SqlExpression><![CDATA[{}]]></SqlExpression></Filter><Action i:nil="true" /></RuleDescription>"#,
        expr
    )
}

// ──────────────────────────── Implementation ────────────────────────────

impl ManagementClient {
    pub fn new(config: ConnectionConfig) -> Self {
        let http = Client::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .unwrap_or_default();
        Self { config, http }
    }

    /// Fetch every entry of an ATOM feed, following `$skip`/`$top` paging.
    ///
    /// Service Bus returns at most 100 entries per request; reading only the
    /// first page silently hides the rest of a large namespace.
    async fn get_atom_feed(&self, path: &str) -> Result<Vec<String>> {
        let mut entries = Vec::new();
        for page in 0..MAX_FEED_PAGES {
            let query = format!("$skip={}&$top={}", page * FEED_PAGE_SIZE, FEED_PAGE_SIZE);
            let xml = self.get_atom_with_query(path, &query).await?;
            let page_entries = extract_entries(&xml);
            let page_len = page_entries.len();
            entries.extend(page_entries);
            if page_len < FEED_PAGE_SIZE {
                return Ok(entries);
            }
        }
        Err(ServiceBusError::Operation(format!(
            "feed '{}' did not end after {} pages",
            path, MAX_FEED_PAGES
        )))
    }

    async fn get_atom(&self, path: &str) -> Result<String> {
        self.get_atom_with_query(path, "").await
    }

    async fn get_atom_with_query(&self, path: &str, extra_query: &str) -> Result<String> {
        let mut url = format!("{}/{}?api-version=2017-04", self.config.endpoint, path);
        if !extra_query.is_empty() {
            url.push('&');
            url.push_str(extra_query);
        }
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

    /// List queues with (active_message_count, dead_letter_message_count) from the same feed.
    pub async fn list_queues_with_counts(&self) -> Result<Vec<(QueueDescription, i64, i64)>> {
        let entries = self.get_atom_feed("$Resources/Queues").await?;
        Ok(parse_queue_feed_with_counts(entries))
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

    pub async fn delete_queue(&self, name: &str) -> Result<()> {
        self.delete_entity(name).await
    }

    // ────────── Topics ──────────

    pub async fn list_topics(&self) -> Result<Vec<TopicDescription>> {
        let entries = self.get_atom_feed("$Resources/Topics").await?;
        Ok(parse_topic_feed(entries))
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

    pub async fn delete_topic(&self, name: &str) -> Result<()> {
        self.delete_entity(name).await
    }

    // ────────── Subscriptions ──────────

    pub async fn list_subscriptions(
        &self,
        topic_name: &str,
    ) -> Result<Vec<SubscriptionDescription>> {
        let entries = self
            .get_atom_feed(&format!("{}/Subscriptions", topic_name))
            .await?;
        Ok(parse_subscription_feed(topic_name, entries))
    }

    /// List subscriptions with (active_message_count, dead_letter_message_count) from the same feed.
    pub async fn list_subscriptions_with_counts(
        &self,
        topic_name: &str,
    ) -> Result<Vec<(SubscriptionDescription, i64, i64)>> {
        let entries = self
            .get_atom_feed(&format!("{}/Subscriptions", topic_name))
            .await?;
        Ok(parse_subscription_feed_with_counts(topic_name, entries))
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

    pub async fn delete_subscription(&self, topic_name: &str, sub_name: &str) -> Result<()> {
        self.delete_entity(&format!("{}/Subscriptions/{}", topic_name, sub_name))
            .await
    }

    pub async fn list_subscription_rules(
        &self,
        topic_name: &str,
        sub_name: &str,
    ) -> Result<Vec<SubscriptionRule>> {
        let entries = self
            .get_atom_feed(&format!("{}/Subscriptions/{}/Rules", topic_name, sub_name))
            .await?;
        Ok(parse_subscription_rule_feed(entries))
    }

    pub async fn upsert_subscription_sql_rule(
        &self,
        topic_name: &str,
        sub_name: &str,
        rule_name: &str,
        sql_expression: &str,
    ) -> Result<()> {
        let trimmed_rule_name = rule_name.trim();
        if trimmed_rule_name.is_empty() {
            return Err(ServiceBusError::Operation(
                "Rule name cannot be empty".to_string(),
            ));
        }

        if sql_expression.trim().is_empty() {
            return Err(ServiceBusError::Operation(
                "SQL filter expression cannot be empty".to_string(),
            ));
        }

        let body = wrap_atom_entry(&subscription_rule_sql_xml(sql_expression.trim()));
        let path = format!(
            "{}/Subscriptions/{}/Rules/{}",
            topic_name, sub_name, trimmed_rule_name
        );
        match self.put_atom(&path, &body).await {
            Ok(_) => {}
            Err(ServiceBusError::Api { status: 409, .. }) => {
                // Rules are create-only in some Service Bus API paths. If the rule
                // already exists, replace it explicitly.
                self.delete_entity(&path).await?;
                self.put_atom(&path, &body).await?;
            }
            Err(e) => return Err(e),
        }
        Ok(())
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
            let val = xml[content_start..content_start + end_pos].trim();
            if val.is_empty() {
                return None;
            }
            return Some(unesc(val));
        }
    }
    None
}

fn extract_title(entry_xml: &str) -> String {
    // Azure ATOM feeds use <title type="text">name</title>, so we must use
    // extract_element (handles attributes) rather than extract_element_value
    // (which requires an exact <title> open tag with no attributes).
    extract_element(entry_xml, "title")
        .map(|s| unesc(s.trim()))
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

fn parse_queue_feed_with_counts(entries: Vec<String>) -> Vec<(QueueDescription, i64, i64)> {
    entries
        .into_iter()
        .map(|e| {
            let desc = parse_queue_from_entry(&e);
            let (active, dlq, _, _, _) = parse_count_details(&e);
            (desc, active, dlq)
        })
        .collect()
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

fn parse_topic_feed(entries: Vec<String>) -> Vec<TopicDescription> {
    entries
        .into_iter()
        .map(|e| parse_topic_from_entry(&e))
        .collect()
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

fn parse_subscription_feed(topic_name: &str, entries: Vec<String>) -> Vec<SubscriptionDescription> {
    entries
        .into_iter()
        .map(|e| parse_subscription_from_entry(topic_name, &e))
        .collect()
}

fn parse_subscription_feed_with_counts(
    topic_name: &str,
    entries: Vec<String>,
) -> Vec<(SubscriptionDescription, i64, i64)> {
    entries
        .into_iter()
        .map(|e| {
            let desc = parse_subscription_from_entry(topic_name, &e);
            let (active, dlq, _, _, _) = parse_count_details(&e);
            (desc, active, dlq)
        })
        .collect()
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

fn parse_subscription_rule_from_entry(entry_xml: &str) -> SubscriptionRule {
    let name = extract_title(entry_xml);
    let filter_kind = parse_rule_filter_kind(entry_xml);
    let sql_expression = match filter_kind {
        RuleFilterKind::Sql => extract_value_any_ns(entry_xml, "SqlExpression")
            .or_else(|| extract_value_any_ns(entry_xml, "Expression"))
            .unwrap_or_default(),
        RuleFilterKind::True => "1=1".to_string(),
        RuleFilterKind::False => "1=0".to_string(),
        RuleFilterKind::Correlation | RuleFilterKind::Unknown => String::new(),
    };

    SubscriptionRule {
        name,
        sql_expression,
        filter_kind,
    }
}

/// Read the filter's `i:type` attribute. `TrueFilter`/`FalseFilter` derive
/// from `SqlFilter` and are checked first.
fn parse_rule_filter_kind(entry_xml: &str) -> RuleFilterKind {
    let filter_open = entry_xml
        .find("<Filter")
        .and_then(|start| {
            entry_xml[start..]
                .find('>')
                .map(|end| &entry_xml[start..start + end])
        })
        .unwrap_or("");
    if filter_open.contains("\"TrueFilter\"") {
        RuleFilterKind::True
    } else if filter_open.contains("\"FalseFilter\"") {
        RuleFilterKind::False
    } else if filter_open.contains("\"CorrelationFilter\"") {
        RuleFilterKind::Correlation
    } else if filter_open.contains("\"SqlFilter\"") {
        RuleFilterKind::Sql
    } else {
        RuleFilterKind::Unknown
    }
}

fn parse_subscription_rule_feed(entries: Vec<String>) -> Vec<SubscriptionRule> {
    entries
        .into_iter()
        .map(|e| parse_subscription_rule_from_entry(&e))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUEUE_ENTRY_PREFIXED: &str = r#"<entry xmlns="http://www.w3.org/2005/Atom"><title type="text">orders&amp;returns</title><content type="application/xml"><QueueDescription xmlns="http://schemas.microsoft.com/netservices/2010/10/servicebus/connect"><LockDuration>PT1M</LockDuration><MaxDeliveryCount>7</MaxDeliveryCount><ForwardTo>a&amp;b</ForwardTo><CountDetails xmlns:d2p1="http://schemas.microsoft.com/netservices/2011/06/servicebus"><d2p1:ActiveMessageCount>12</d2p1:ActiveMessageCount><d2p1:DeadLetterMessageCount>3</d2p1:DeadLetterMessageCount><d2p1:ScheduledMessageCount>1</d2p1:ScheduledMessageCount></CountDetails></QueueDescription></content></entry>"#;

    const QUEUE_ENTRY_UNPREFIXED: &str = r#"<entry><title type="text">q2</title><content><QueueDescription><CountDetails><ActiveMessageCount>5</ActiveMessageCount><DeadLetterMessageCount>0</DeadLetterMessageCount></CountDetails></QueueDescription></content></entry>"#;

    fn rule_entry(filter: &str) -> String {
        format!(
            r#"<entry><title type="text">$Default</title><content><RuleDescription>{}</RuleDescription></content></entry>"#,
            filter
        )
    }

    #[test]
    fn count_details_parse_with_and_without_namespace_prefix() {
        let (active, dlq, scheduled, _, _) = parse_count_details(QUEUE_ENTRY_PREFIXED);
        assert_eq!((active, dlq, scheduled), (12, 3, 1));
        let (active, dlq, _, _, _) = parse_count_details(QUEUE_ENTRY_UNPREFIXED);
        assert_eq!((active, dlq), (5, 0));
    }

    #[test]
    fn entity_text_is_unescaped() {
        let q = parse_queue_from_entry(QUEUE_ENTRY_PREFIXED);
        assert_eq!(q.name, "orders&returns");
        assert_eq!(q.forward_to.as_deref(), Some("a&b"));
        assert_eq!(q.max_delivery_count, Some(7));
    }

    #[test]
    fn feed_entries_are_split_per_entity() {
        let feed = format!(
            "<feed>{}{}</feed>",
            QUEUE_ENTRY_PREFIXED, QUEUE_ENTRY_UNPREFIXED
        );
        let parsed = parse_queue_feed_with_counts(extract_entries(&feed));
        let names: Vec<&str> = parsed.iter().map(|(q, _, _)| q.name.as_str()).collect();
        assert_eq!(names, vec!["orders&returns", "q2"]);
    }

    #[test]
    fn sql_filter_with_comparison_operators_round_trips() {
        let entry = rule_entry(
            r#"<Filter i:type="SqlFilter"><SqlExpression>Priority &gt; 5 AND Region &lt;&gt; 'EU'</SqlExpression></Filter>"#,
        );
        let rule = parse_subscription_rule_from_entry(&entry);
        assert_eq!(rule.filter_kind, RuleFilterKind::Sql);
        assert_eq!(rule.sql_expression, "Priority > 5 AND Region <> 'EU'");
        // Written back inside CDATA, the operators stay literal.
        assert!(subscription_rule_sql_xml(&rule.sql_expression)
            .contains("<![CDATA[Priority > 5 AND Region <> 'EU']]>"));
    }

    #[test]
    fn correlation_filters_are_not_mistaken_for_sql() {
        let entry = rule_entry(
            r#"<Filter i:type="CorrelationFilter"><CorrelationId>abc</CorrelationId></Filter>"#,
        );
        let rule = parse_subscription_rule_from_entry(&entry);
        assert_eq!(rule.filter_kind, RuleFilterKind::Correlation);
        assert!(!rule.filter_kind.is_sql_editable());
        assert!(rule.sql_expression.is_empty());
    }

    #[test]
    fn true_filter_is_editable_as_one_equals_one() {
        let entry = rule_entry(
            r#"<Filter i:type="TrueFilter"><SqlExpression>1=1</SqlExpression></Filter>"#,
        );
        let rule = parse_subscription_rule_from_entry(&entry);
        assert_eq!(rule.filter_kind, RuleFilterKind::True);
        assert_eq!(rule.sql_expression, "1=1");
    }

    #[test]
    fn entity_xml_escapes_user_values() {
        let desc = QueueDescription {
            name: "q".into(),
            forward_to: Some("a<b>&c".into()),
            ..Default::default()
        };
        let xml = queue_description_xml(&desc);
        assert!(xml.contains("<ForwardTo>a&lt;b&gt;&amp;c</ForwardTo>"));
    }

    #[test]
    fn cdata_terminator_in_sql_is_split() {
        assert!(!to_cdata_safe("x ]]> y").contains("x ]]> y"));
    }
}
