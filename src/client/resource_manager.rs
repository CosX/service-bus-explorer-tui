use crate::client::error::ArmError;
use crate::client::models::{EntityMetrics, MetricsResponse};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

/// Per-request timeout for Azure Resource Manager calls.
const ARM_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// The only host ARM bearer tokens are ever sent to.
const ARM_HOST: &str = "management.azure.com";
const ARM_ENDPOINT: &str = "https://management.azure.com";

/// Upper bound on followed `nextLink`s, so a server bug cannot loop forever.
const MAX_ARM_PAGES: usize = 1000;

/// Concurrent per-subscription namespace listings during discovery.
const DISCOVERY_CONCURRENCY: usize = 8;

/// How often discovery checks the cancel flag while waiting on requests.
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Azure subscription returned from ARM API.
#[derive(Debug, Clone, Deserialize)]
pub struct Subscription {
    #[serde(rename = "subscriptionId")]
    pub subscription_id: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    pub state: String,
}

/// Azure Service Bus namespace resource.
#[derive(Debug, Clone, Deserialize)]
pub struct NamespaceResource {
    pub id: String,
    pub name: String,
    pub location: String,
    pub properties: NamespaceProperties,
}

/// Namespace properties from ARM.
#[derive(Debug, Clone, Deserialize)]
pub struct NamespaceProperties {
    #[serde(rename = "serviceBusEndpoint")]
    pub service_bus_endpoint: String,
    pub status: String,
}

/// Discovered namespace with enriched metadata.
#[derive(Debug, Clone)]
pub struct DiscoveredNamespace {
    pub fqdn: String,
    pub name: String,
    pub subscription_name: String,
    pub resource_id: String,
    pub location: String,
    pub status: String,
}

/// Result of namespace discovery operation.
#[derive(Debug, Clone)]
pub struct DiscoveryResult {
    pub namespaces: Vec<DiscoveredNamespace>,
    pub errors: Vec<String>,
}

/// Azure Resource Manager client for discovering Service Bus namespaces.
#[derive(Clone)]
pub struct ResourceManagerClient {
    http_client: reqwest::Client,
    credential: Arc<dyn azure_core::credentials::TokenCredential>,
}

/// Generic ARM list page (`value` + optional `nextLink`).
#[derive(Debug, Deserialize)]
struct Page<T> {
    value: Vec<T>,
    #[serde(rename = "nextLink")]
    next_link: Option<String>,
}

impl ResourceManagerClient {
    /// Create a new Resource Manager client.
    pub fn new(credential: Arc<dyn azure_core::credentials::TokenCredential>) -> Self {
        // Without a timeout a stalled ARM call leaves the UI resolving forever.
        let http_client = reqwest::Client::builder()
            .timeout(ARM_REQUEST_TIMEOUT)
            .build()
            .unwrap_or_default();
        Self {
            http_client,
            credential,
        }
    }

    /// Get an Azure Resource Manager bearer token.
    async fn get_token(&self) -> Result<String, ArmError> {
        let token = self
            .credential
            .get_token(&["https://management.azure.com/.default"], None)
            .await
            .map_err(|e| ArmError::Auth(e.to_string()))?;

        Ok(token.token.secret().to_string())
    }

    /// GET an ARM list endpoint and follow every `nextLink`.
    ///
    /// ARM pages large result sets; stopping at the first page silently hides
    /// resources in larger tenants. The bearer token is only ever sent to
    /// `https://management.azure.com`, whatever host a `nextLink` names.
    async fn get_all_pages<T: DeserializeOwned>(
        &self,
        first_url: String,
    ) -> Result<Vec<T>, ArmError> {
        let token = self.get_token().await?;
        let mut url = first_url;
        let mut items = Vec::new();

        for _ in 0..MAX_ARM_PAGES {
            let response = self
                .http_client
                .get(&url)
                .bearer_auth(&token)
                .send()
                .await?;

            let status = response.status();
            if !status.is_success() {
                let body = response
                    .text()
                    .await
                    .unwrap_or_else(|_| String::from("(no body)"));
                return Err(ArmError::Status {
                    status: status.as_u16(),
                    body,
                });
            }

            let page: Page<T> = response
                .json()
                .await
                .map_err(|e| ArmError::Parse(e.to_string()))?;
            items.extend(page.value);

            match page.next_link {
                Some(next) if !next.is_empty() => {
                    if !is_trusted_arm_url(&next) {
                        return Err(ArmError::UntrustedLink(next));
                    }
                    url = next;
                }
                _ => return Ok(items),
            }
        }

        Err(ArmError::TooManyPages(MAX_ARM_PAGES))
    }

    /// List all accessible, enabled Azure subscriptions.
    pub async fn list_subscriptions(&self) -> Result<Vec<Subscription>, ArmError> {
        let subscriptions: Vec<Subscription> = self
            .get_all_pages(format!(
                "{}/subscriptions?api-version=2020-01-01",
                ARM_ENDPOINT
            ))
            .await?;

        Ok(subscriptions
            .into_iter()
            .filter(|s| s.state.eq_ignore_ascii_case("enabled"))
            .collect())
    }

    /// List Service Bus namespaces in a subscription.
    pub async fn list_namespaces(
        &self,
        subscription_id: &str,
    ) -> Result<Vec<NamespaceResource>, ArmError> {
        self.get_all_pages(format!(
            "{}/subscriptions/{}/providers/Microsoft.ServiceBus/namespaces?api-version=2021-11-01",
            ARM_ENDPOINT, subscription_id
        ))
        .await
    }

    /// Discover all Service Bus namespaces across all subscriptions.
    /// Returns both successful discoveries and per-subscription errors.
    ///
    /// Subscriptions are queried with bounded concurrency so large tenants do
    /// not trip ARM throttling, and outstanding requests are aborted on cancel.
    pub async fn discover_namespaces(&self, cancel: Arc<AtomicBool>) -> DiscoveryResult {
        let empty = || DiscoveryResult {
            namespaces: Vec::new(),
            errors: Vec::new(),
        };

        if cancel.load(Ordering::Relaxed) {
            return empty();
        }

        let subscriptions = match self.list_subscriptions().await {
            Ok(subs) => subs,
            Err(e) => {
                return DiscoveryResult {
                    namespaces: Vec::new(),
                    errors: vec![format!("Failed to list subscriptions: {}", e)],
                };
            }
        };

        if cancel.load(Ordering::Relaxed) {
            return empty();
        }

        if subscriptions.is_empty() {
            return DiscoveryResult {
                namespaces: Vec::new(),
                errors: vec!["No enabled Azure subscriptions found".to_string()],
            };
        }

        let limit = Arc::new(Semaphore::new(DISCOVERY_CONCURRENCY));
        let mut tasks = JoinSet::new();
        for sub in subscriptions {
            let client = self.clone();
            let limit = Arc::clone(&limit);
            tasks.spawn(async move {
                // The semaphore is never closed, so acquire only fails if it
                // were; treat that as "run anyway" rather than dropping work.
                let _permit = limit.acquire_owned().await.ok();
                let result = client.list_namespaces(&sub.subscription_id).await;
                (sub.display_name, result)
            });
        }

        let mut all_namespaces = Vec::new();
        let mut errors = Vec::new();

        loop {
            tokio::select! {
                joined = tasks.join_next() => {
                    let Some(joined) = joined else { break };
                    match joined {
                        Ok((sub_name, Ok(namespaces))) => {
                            for ns in namespaces {
                                let fqdn =
                                    extract_fqdn_from_endpoint(&ns.properties.service_bus_endpoint);
                                all_namespaces.push(DiscoveredNamespace {
                                    fqdn,
                                    name: ns.name,
                                    subscription_name: sub_name.clone(),
                                    resource_id: ns.id,
                                    location: ns.location,
                                    status: ns.properties.status,
                                });
                            }
                        }
                        Ok((sub_name, Err(e))) => {
                            errors.push(format!("Subscription '{}': {}", sub_name, e));
                        }
                        Err(e) => errors.push(format!("Task join error: {}", e)),
                    }
                }
                _ = tokio::time::sleep(CANCEL_POLL_INTERVAL) => {
                    if cancel.load(Ordering::Relaxed) {
                        tasks.abort_all();
                        return empty();
                    }
                }
            }
        }

        all_namespaces.sort_by(|a, b| {
            a.subscription_name
                .cmp(&b.subscription_name)
                .then_with(|| a.name.cmp(&b.name))
        });

        DiscoveryResult {
            namespaces: all_namespaces,
            errors,
        }
    }

    /// Resolve a namespace FQDN to its ARM resource ID.
    ///
    /// Iterates subscriptions and their namespaces to find the matching FQDN.
    /// Returns the full ARM resource path, e.g.
    /// `/subscriptions/{sub}/resourceGroups/{rg}/providers/Microsoft.ServiceBus/namespaces/{name}`.
    pub async fn resolve_namespace_resource_id(&self, fqdn: &str) -> Result<String, ArmError> {
        let subscriptions = self.list_subscriptions().await?;
        let mut listing_errors = Vec::new();

        for sub in &subscriptions {
            let namespaces = match self.list_namespaces(&sub.subscription_id).await {
                Ok(ns) => ns,
                Err(e) => {
                    listing_errors.push(format!("{}: {}", sub.display_name, e));
                    continue;
                }
            };
            for ns in namespaces {
                let ns_fqdn = extract_fqdn_from_endpoint(&ns.properties.service_bus_endpoint);
                if ns_fqdn.eq_ignore_ascii_case(fqdn) {
                    return Ok(ns.id);
                }
            }
        }

        // Distinguish "not found" from "could not look" — the latter is a
        // permissions problem on the ARM side, not a missing namespace.
        if !listing_errors.is_empty() && listing_errors.len() == subscriptions.len() {
            return Err(ArmError::ListingFailed {
                count: subscriptions.len(),
                details: listing_errors.join("; "),
            });
        }

        Err(ArmError::NotFound(format!(
            "'{}' not found in {} accessible subscription(s); Azure Monitor needs \
             Microsoft.ServiceBus/namespaces/read on the namespace",
            fqdn,
            subscriptions.len()
        )))
    }

    /// Query Azure Monitor for entity metrics.
    pub async fn query_entity_metrics(
        &self,
        resource_id: &str,
        entity_name: &str,
        timespan: &str,
        interval: &str,
    ) -> Result<EntityMetrics, ArmError> {
        let token = self.get_token().await?;
        let filter = format!("EntityName eq '{}'", entity_name);
        let url = format!(
            "{}{}/providers/microsoft.insights/metrics",
            ARM_ENDPOINT, resource_id
        );

        // Fetch gauge metrics (average aggregation) and throughput metrics (total aggregation)
        // in parallel since Azure Monitor requires separate aggregation types.
        let gauge_request = self
            .http_client
            .get(&url)
            .bearer_auth(&token)
            .query(&[
                ("api-version", "2023-10-01"),
                (
                    "metricnames",
                    "ActiveMessages,DeadletteredMessages,ScheduledMessages",
                ),
                ("aggregation", "average"),
                ("interval", interval),
                ("timespan", timespan),
                ("$filter", &filter),
            ])
            .send();

        let throughput_request = self
            .http_client
            .get(&url)
            .bearer_auth(&token)
            .query(&[
                ("api-version", "2023-10-01"),
                ("metricnames", "IncomingMessages,OutgoingMessages"),
                ("aggregation", "total"),
                ("interval", interval),
                ("timespan", timespan),
                ("$filter", &filter),
            ])
            .send();

        let (gauge_resp, throughput_resp) = tokio::join!(gauge_request, throughput_request);

        let gauge_resp = gauge_resp?;
        let throughput_resp = throughput_resp?;

        let gauge_resp = ensure_success(gauge_resp).await?;
        let throughput_resp = ensure_success(throughput_resp).await?;

        let gauge_parsed: MetricsResponse = gauge_resp
            .json()
            .await
            .map_err(|e| ArmError::Parse(format!("gauge metrics: {}", e)))?;

        let throughput_parsed: MetricsResponse = throughput_resp
            .json()
            .await
            .map_err(|e| ArmError::Parse(format!("throughput metrics: {}", e)))?;

        let mut active = Vec::new();
        let mut dlq = Vec::new();
        let mut scheduled = Vec::new();
        let mut incoming = Vec::new();
        let mut outgoing = Vec::new();

        let extract_data =
            |metric: &crate::client::models::MetricValue, use_total: bool| -> Vec<u64> {
                metric
                    .timeseries
                    .first()
                    .map(|ts| {
                        ts.data
                            .iter()
                            .map(|dp| {
                                let val = if use_total {
                                    dp.total.unwrap_or(0.0)
                                } else {
                                    dp.average.unwrap_or(0.0)
                                };
                                val as u64
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            };

        for metric in &gauge_parsed.value {
            match metric.name.value.as_str() {
                "ActiveMessages" => active = extract_data(metric, false),
                "DeadletteredMessages" => dlq = extract_data(metric, false),
                "ScheduledMessages" => scheduled = extract_data(metric, false),
                _ => {}
            }
        }

        for metric in &throughput_parsed.value {
            match metric.name.value.as_str() {
                "IncomingMessages" => incoming = extract_data(metric, true),
                "OutgoingMessages" => outgoing = extract_data(metric, true),
                _ => {}
            }
        }

        Ok(EntityMetrics {
            active_messages: active,
            dead_letter_messages: dlq,
            incoming_messages: incoming,
            outgoing_messages: outgoing,
            scheduled_messages: scheduled,
            entity_name: entity_name.to_string(),
            fetched_at: Instant::now(),
        })
    }
}

/// Turn a non-2xx ARM response into an [`ArmError::Status`].
async fn ensure_success(response: reqwest::Response) -> Result<reqwest::Response, ArmError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response
        .text()
        .await
        .unwrap_or_else(|_| String::from("(no body)"));
    Err(ArmError::Status {
        status: status.as_u16(),
        body,
    })
}

/// Only `https://management.azure.com/...` may receive an ARM bearer token.
fn is_trusted_arm_url(url: &str) -> bool {
    reqwest::Url::parse(url)
        .map(|u| u.scheme() == "https" && u.host_str() == Some(ARM_HOST) && u.port().is_none())
        .unwrap_or(false)
}

/// Extract FQDN from Azure Service Bus endpoint URL.
/// Example: "https://mynamespace.servicebus.windows.net:443/" -> "mynamespace.servicebus.windows.net"
fn extract_fqdn_from_endpoint(endpoint: &str) -> String {
    let trimmed = endpoint
        .trim_start_matches("https://")
        .trim_start_matches("http://");

    if let Some(colon_idx) = trimmed.find(':') {
        trimmed[..colon_idx].to_string()
    } else if let Some(slash_idx) = trimmed.find('/') {
        trimmed[..slash_idx].to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_fqdn() {
        assert_eq!(
            extract_fqdn_from_endpoint("https://myns.servicebus.windows.net:443/"),
            "myns.servicebus.windows.net"
        );
        assert_eq!(
            extract_fqdn_from_endpoint("https://myns.servicebus.windows.net/"),
            "myns.servicebus.windows.net"
        );
        assert_eq!(
            extract_fqdn_from_endpoint("myns.servicebus.windows.net"),
            "myns.servicebus.windows.net"
        );
    }

    #[test]
    fn paging_links_are_only_followed_to_arm() {
        assert!(is_trusted_arm_url(
            "https://management.azure.com/subscriptions?api-version=2020-01-01&$skiptoken=abc"
        ));
        assert!(!is_trusted_arm_url(
            "http://management.azure.com/subscriptions"
        ));
        assert!(!is_trusted_arm_url("https://evil.example/subscriptions"));
        assert!(!is_trusted_arm_url(
            "https://management.azure.com.evil.example/subscriptions"
        ));
        assert!(!is_trusted_arm_url("https://management.azure.com:8443/x"));
        assert!(!is_trusted_arm_url("not a url"));
    }

    #[test]
    fn page_without_next_link_deserializes() {
        let page: Page<Subscription> = serde_json::from_str(
            r#"{"value":[{"subscriptionId":"s1","displayName":"One","state":"Enabled"}]}"#,
        )
        .expect("valid page");
        assert_eq!(page.value.len(), 1);
        assert!(page.next_link.is_none());
    }
}
