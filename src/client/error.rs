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
