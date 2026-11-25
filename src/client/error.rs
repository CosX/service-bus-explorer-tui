use thiserror::Error;

#[derive(Error, Debug)]
pub enum ServiceBusError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("XML parsing error: {0}")]
    Xml(#[from] quick_xml::DeError),

    #[error("Authentication error: {0}")]
    Auth(String),

    #[error("Service Bus returned {status}: {body}")]
    Api { status: u16, body: String },

    #[error("Connection string is invalid: {0}")]
    InvalidConnectionString(String),

    #[error("Entity not found: {0}")]
    NotFound(String),

    #[error("Operation failed: {0}")]
    Operation(String),

    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, ServiceBusError>;

/// Errors from Azure Resource Manager / Azure Monitor calls.
///
/// Typed so callers can tell a permission problem from throttling or a
/// network failure without parsing message text.
#[derive(Error, Debug)]
pub enum ArmError {
    #[error("failed to get ARM token: {0}")]
    Auth(String),

    #[error("ARM request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("ARM returned {status}: {body}")]
    Status { status: u16, body: String },

    #[error("unexpected ARM response: {0}")]
    Parse(String),

    #[error("refusing to follow ARM paging link to untrusted host: {0}")]
    UntrustedLink(String),

    #[error("ARM paging did not terminate after {0} pages")]
    TooManyPages(usize),

    #[error("{0}")]
    NotFound(String),

    #[error("could not list namespaces in any of {count} subscription(s): {details}")]
    ListingFailed { count: usize, details: String },
}

impl ArmError {
    /// True for errors that retrying soon is unlikely to fix
    /// (permissions, missing resources), as opposed to throttling or transport.
    pub fn is_permanent(&self) -> bool {
        match self {
            ArmError::Status { status, .. } => matches!(status, 400 | 401 | 403 | 404),
            ArmError::NotFound(_) | ArmError::ListingFailed { .. } => true,
            _ => false,
        }
    }
}
