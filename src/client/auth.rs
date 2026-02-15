use std::sync::Arc;

use base64::Engine;
use chrono::Utc;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use azure_core::credentials::TokenCredential;

use super::error::{Result, ServiceBusError};

type HmacSha256 = Hmac<Sha256>;

/// The Service Bus token audience used for Azure AD authentication.
const SERVICE_BUS_SCOPE: &str = "https://servicebus.azure.net/.default";

/// Authentication mode — either SAS key-based or Azure AD (Microsoft Entra ID).
#[derive(Clone)]
pub enum AuthMode {
    Sas {
        shared_access_key_name: String,
        shared_access_key: String,
    },
    AzureAd {
        credential: Arc<dyn TokenCredential>,
    },
}

impl std::fmt::Debug for AuthMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sas {
                shared_access_key_name,
                ..
            } => f
                .debug_struct("Sas")
                .field("shared_access_key_name", shared_access_key_name)
                .finish(),
            Self::AzureAd { .. } => f.write_str("AzureAd"),
        }
    }
}

/// Parsed components from a Service Bus connection string or Azure AD config.
#[derive(Debug, Clone)]
pub struct ConnectionConfig {
    pub namespace: String,
    pub endpoint: String,
    pub auth_mode: AuthMode,
}

impl ConnectionConfig {
    /// Parse a standard Service Bus connection string (SAS auth).
    ///
    /// Expected format:
    /// `Endpoint=sb://<namespace>.servicebus.windows.net/;SharedAccessKeyName=<name>;SharedAccessKey=<key>`
    pub fn from_connection_string(conn_str: &str) -> Result<Self> {
        let mut endpoint = None;
        let mut key_name = None;
        let mut key = None;

        for part in conn_str.split(';') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            if let Some((k, v)) = part.split_once('=') {
                match k.trim() {
                    "Endpoint" => endpoint = Some(v.trim().to_string()),
                    "SharedAccessKeyName" => key_name = Some(v.trim().to_string()),
                    // Key value may contain '=' (base64 padding)
                    "SharedAccessKey" => {
                        let idx = part.find('=').unwrap();
                        key = Some(part[idx + 1..].trim().to_string());
                    }
                    _ => {}
                }
            }
        }

        let endpoint = endpoint
            .ok_or_else(|| ServiceBusError::InvalidConnectionString("missing Endpoint".into()))?;
        let key_name = key_name.ok_or_else(|| {
            ServiceBusError::InvalidConnectionString("missing SharedAccessKeyName".into())
        })?;
        let key = key.ok_or_else(|| {
            ServiceBusError::InvalidConnectionString("missing SharedAccessKey".into())
        })?;

        // Extract namespace from endpoint like sb://mynamespace.servicebus.windows.net/
        let namespace = endpoint
            .trim_start_matches("sb://")
            .trim_end_matches('/')
            .to_string();

        // Normalize endpoint to https://
        let https_endpoint = format!("https://{}", namespace);

        Ok(Self {
            namespace,
            endpoint: https_endpoint,
            auth_mode: AuthMode::Sas {
                shared_access_key_name: key_name,
                shared_access_key: key,
            },
        })
    }

    /// Create a config for Azure AD (Microsoft Entra ID) authentication.
    ///
    /// `namespace` should be the fully-qualified namespace, e.g.
    /// `mynamespace.servicebus.windows.net`.
    pub fn from_azure_ad(namespace: &str, credential: Arc<dyn TokenCredential>) -> Self {
        let namespace = namespace
            .trim_start_matches("sb://")
            .trim_end_matches('/')
            .to_string();
        let endpoint = format!("https://{}", namespace);
        Self {
            namespace,
            endpoint,
            auth_mode: AuthMode::AzureAd { credential },
        }
    }

    /// Returns `true` if using Azure AD authentication.
    pub fn is_azure_ad(&self) -> bool {
        matches!(self.auth_mode, AuthMode::AzureAd { .. })
    }

    /// Generate a SAS token for the given resource URI, valid for `validity_secs`.
    fn generate_sas_token(
        key_name: &str,
        key: &str,
        resource_uri: &str,
        validity_secs: u64,
    ) -> Result<String> {
        let encoded_uri = urlencoding::encode(resource_uri).to_lowercase();
        let expiry = Utc::now().timestamp() as u64 + validity_secs;
        let string_to_sign = format!("{}\n{}", encoded_uri, expiry);

        let mut mac = HmacSha256::new_from_slice(key.as_bytes())
            .map_err(|e| ServiceBusError::Auth(format!("HMAC key error: {}", e)))?;
        mac.update(string_to_sign.as_bytes());
        let signature =
            base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
        let encoded_signature = urlencoding::encode(&signature);

        Ok(format!(
            "SharedAccessSignature sr={}&sig={}&se={}&skn={}",
            encoded_uri, encoded_signature, expiry, key_name
        ))
    }

    /// Acquire a Bearer token from Azure AD.
    async fn get_azure_ad_token(credential: &dyn TokenCredential) -> Result<String> {
        let token = credential
            .get_token(&[SERVICE_BUS_SCOPE])
            .await
            .map_err(|e| ServiceBusError::Auth(format!("Azure AD token error: {}", e)))?;
        Ok(format!("Bearer {}", token.token.secret()))
    }

    /// Generate an authorization header scoped to the namespace root.
    ///
    /// For SAS: generates an HMAC-SHA256 token valid for 1 hour.
    /// For Azure AD: acquires a Bearer token from the credential chain.
    pub async fn namespace_token(&self) -> Result<String> {
        match &self.auth_mode {
            AuthMode::Sas {
                shared_access_key_name,
                shared_access_key,
            } => Self::generate_sas_token(
                shared_access_key_name,
                shared_access_key,
                &self.endpoint,
                3600,
            ),
            AuthMode::AzureAd { credential } => Self::get_azure_ad_token(credential.as_ref()).await,
        }
    }

    /// Generate an authorization header scoped to a specific entity.
    ///
    /// For SAS: generates an HMAC-SHA256 token valid for 1 hour.
    /// For Azure AD: acquires a Bearer token (scope is namespace-level
    /// regardless, but the API authorization matches the entity).
    pub async fn entity_token(&self, entity_path: &str) -> Result<String> {
        match &self.auth_mode {
            AuthMode::Sas {
                shared_access_key_name,
                shared_access_key,
            } => {
                let uri = format!("{}/{}", self.endpoint, entity_path);
                Self::generate_sas_token(shared_access_key_name, shared_access_key, &uri, 3600)
            }
            AuthMode::AzureAd { credential } => Self::get_azure_ad_token(credential.as_ref()).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_connection_string() {
        let cs = "Endpoint=sb://myns.servicebus.windows.net/;SharedAccessKeyName=RootManageSharedAccessKey;SharedAccessKey=abc123def456==";
        let cfg = ConnectionConfig::from_connection_string(cs).unwrap();
        assert_eq!(cfg.namespace, "myns.servicebus.windows.net");
        assert_eq!(cfg.endpoint, "https://myns.servicebus.windows.net");
        assert!(matches!(
            cfg.auth_mode,
            AuthMode::Sas { ref shared_access_key_name, ref shared_access_key }
            if shared_access_key_name == "RootManageSharedAccessKey"
                && shared_access_key == "abc123def456=="
        ));
    }

    #[test]
    fn parse_missing_endpoint() {
        let cs = "SharedAccessKeyName=name;SharedAccessKey=key";
        assert!(ConnectionConfig::from_connection_string(cs).is_err());
    }

    #[tokio::test]
    async fn sas_token_format() {
        let cs = "Endpoint=sb://myns.servicebus.windows.net/;SharedAccessKeyName=RootManageSharedAccessKey;SharedAccessKey=dGVzdGtleQ==";
        let cfg = ConnectionConfig::from_connection_string(cs).unwrap();
        let token = cfg.namespace_token().await.unwrap();
        assert!(token.starts_with("SharedAccessSignature sr="));
        assert!(token.contains("&sig="));
        assert!(token.contains("&se="));
        assert!(token.contains("&skn=RootManageSharedAccessKey"));
    }
}
