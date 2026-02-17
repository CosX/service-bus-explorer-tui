use serde::Deserialize;
use std::sync::Arc;

/// Azure subscription returned from ARM API.
#[derive(Debug, Clone, Deserialize)]
pub struct Subscription {
    #[serde(rename = "subscriptionId")]
    pub subscription_id: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    pub state: String,
}

/// List wrapper for subscriptions.
#[derive(Debug, Deserialize)]
struct SubscriptionListResponse {
    value: Vec<Subscription>,
}

/// Azure Service Bus namespace resource.
#[derive(Debug, Clone, Deserialize)]
pub struct NamespaceResource {
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

/// List wrapper for namespaces.
#[derive(Debug, Deserialize)]
struct NamespaceListResponse {
    value: Vec<NamespaceResource>,
}

/// Discovered namespace with enriched metadata.
#[derive(Debug, Clone)]
pub struct DiscoveredNamespace {
    pub fqdn: String,
    pub name: String,
    pub subscription_name: String,
    pub subscription_id: String,
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

impl ResourceManagerClient {
    /// Create a new Resource Manager client.
    pub fn new(credential: Arc<dyn azure_core::credentials::TokenCredential>) -> Self {
        Self {
            http_client: reqwest::Client::new(),
            credential,
        }
    }

    /// Get an Azure Resource Manager bearer token.
    async fn get_token(&self) -> Result<String, String> {
        let token = self
            .credential
            .get_token(&["https://management.azure.com/.default"])
            .await
            .map_err(|e| format!("Failed to get ARM token: {}", e))?;

        Ok(token.token.secret().to_string())
    }

    /// List all accessible Azure subscriptions.
    pub async fn list_subscriptions(&self) -> Result<Vec<Subscription>, String> {
        let token = self.get_token().await?;
        let url = "https://management.azure.com/subscriptions?api-version=2020-01-01";

        let response = self
            .http_client
            .get(url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| format!("Failed to list subscriptions: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("(no body)"));
            return Err(format!(
                "Subscription list failed ({}): {}",
                status, body
            ));
        }

        let parsed: SubscriptionListResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse subscription list: {}", e))?;

        // Filter only active subscriptions
        let active: Vec<Subscription> = parsed
            .value
            .into_iter()
            .filter(|s| s.state.to_lowercase() == "enabled")
            .collect();

        Ok(active)
    }

    /// List Service Bus namespaces in a subscription.
    pub async fn list_namespaces(
        &self,
        subscription_id: &str,
    ) -> Result<Vec<NamespaceResource>, String> {
        let token = self.get_token().await?;
        let url = format!(
            "https://management.azure.com/subscriptions/{}/providers/Microsoft.ServiceBus/namespaces?api-version=2021-11-01",
            subscription_id
        );

        let response = self
            .http_client
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| format!("Failed to list namespaces: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("(no body)"));
            return Err(format!("Namespace list failed ({}): {}", status, body));
        }

        let parsed: NamespaceListResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse namespace list: {}", e))?;

        Ok(parsed.value)
    }

    /// Discover all Service Bus namespaces across all subscriptions.
    /// Returns both successful discoveries and per-subscription errors.
    pub async fn discover_namespaces(&self) -> DiscoveryResult {
        let mut all_namespaces = Vec::new();
        let mut errors = Vec::new();

        // Get subscriptions
        let subscriptions = match self.list_subscriptions().await {
            Ok(subs) => subs,
            Err(e) => {
                return DiscoveryResult {
                    namespaces: Vec::new(),
                    errors: vec![format!("Failed to list subscriptions: {}", e)],
                };
            }
        };

        if subscriptions.is_empty() {
            return DiscoveryResult {
                namespaces: Vec::new(),
                errors: vec!["No enabled Azure subscriptions found".to_string()],
            };
        }

        // Query each subscription in parallel
        let mut handles = Vec::with_capacity(subscriptions.len());
        for sub in subscriptions {
            let client = self.clone();
            let subscription_id = sub.subscription_id.clone();
            let subscription_name = sub.display_name.clone();

            handles.push(tokio::spawn(async move {
                (
                    subscription_name.clone(),
                    subscription_id.clone(),
                    client.list_namespaces(&subscription_id).await,
                )
            }));
        }

        // Collect results
        for handle in handles {
            match handle.await {
                Ok((sub_name, sub_id, Ok(namespaces))) => {
                    for ns in namespaces {
                        // Extract FQDN from serviceBusEndpoint (e.g., "https://mynamespace.servicebus.windows.net:443/")
                        let fqdn = extract_fqdn_from_endpoint(&ns.properties.service_bus_endpoint);

                        all_namespaces.push(DiscoveredNamespace {
                            fqdn,
                            name: ns.name,
                            subscription_name: sub_name.clone(),
                            subscription_id: sub_id.clone(),
                            location: ns.location,
                            status: ns.properties.status,
                        });
                    }
                }
                Ok((sub_name, _sub_id, Err(e))) => {
                    errors.push(format!("Subscription '{}': {}", sub_name, e));
                }
                Err(e) => {
                    errors.push(format!("Task join error: {}", e));
                }
            }
        }

        // Sort by subscription name, then namespace name
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
}
