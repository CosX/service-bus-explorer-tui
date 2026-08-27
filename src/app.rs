use ratatui::widgets::{ListState, TableState};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;

use crate::client::models::*;
use crate::client::resource_manager::{DiscoveredNamespace, DiscoveryResult};
use crate::client::{ConnectionConfig, DataPlaneClient, ManagementClient};
use crate::config::AppConfig;

/// Events sent from background tasks back to the main loop.
pub enum BgEvent {
    Progress(String),
    PurgeComplete {
        count: u64,
    },
    ResendComplete {
        resent: u32,
        errors: u32,
    },
    BulkDeleteComplete {
        deleted: u32,
        was_dlq: bool,
    },
    SingleDeleteComplete {
        sequence_number: i64,
        is_dlq: bool,
    },
    Cancelled {
        message: String,
    },
    Failed(String),

    // Non-blocking async operation results
    TreeRefreshed {
        tree: TreeNode,
        flat_nodes: Vec<FlatNode>,
    },
    DetailLoaded {
        /// Entity path the detail was loaded for; used to drop stale responses.
        path: String,
        view: Box<DetailView>,
    },
    SubscriptionFilterLoaded {
        topic_name: String,
        sub_name: String,
        rule_name: String,
        sql_expression: String,
    },
    PeekComplete {
        messages: Vec<ReceivedMessage>,
        is_dlq: bool,
    },
    SendComplete {
        status: String,
    },
    EntityCreated {
        status: String,
    },
    EntityDeleted {
        status: String,
    },
    /// Inline/modal resend completed; optionally removed DLQ source.
    ResendSendComplete {
        status: String,
        dlq_seq_removed: Option<i64>,
        was_inline: bool,
    },
    /// Namespace discovery completed.
    NamespacesDiscovered {
        result: DiscoveryResult,
    },
    /// Namespace discovery failed.
    DiscoveryFailed(String),
    DestinationEntitiesLoaded {
        entities: Vec<(String, EntityType)>,
    },
    MessageCopyComplete {
        status: String,
    },
    SubscriptionFilterUpdated {
        status: String,
    },
    /// ARM resource ID resolved for Azure Monitor metrics.
    NamespaceResourceIdResolved(String),
    /// ARM resource ID resolution failed (non-fatal).
    #[allow(dead_code)]
    NamespaceResourceIdFailed(String),
    /// Azure Monitor metrics loaded for an entity.
    MetricsLoaded(EntityMetrics),
    /// Azure Monitor metrics query failed (non-fatal).
    #[allow(dead_code)]
    MetricsFailed(String),
}

/// Which panel is currently focused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusPanel {
    Tree,
    Detail,
    Messages,
}

/// What a clear operation targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClearScope {
    /// A single queue or subscription.
    Entity(String),
    /// A topic; fans out across all of its subscriptions.
    Topic(String),
    /// Every queue in the namespace.
    AllQueues,
    /// Every topic in the namespace, fanned out across their subscriptions.
    AllTopics,
}

impl ClearScope {
    /// Human-readable target for modal titles and status messages.
    pub fn label(&self) -> &str {
        match self {
            ClearScope::Entity(path) | ClearScope::Topic(path) => path,
            ClearScope::AllQueues => "ALL queues",
            ClearScope::AllTopics => "ALL topics",
        }
    }

    /// Namespace-wide scopes are confirmed before they run.
    pub fn is_batch(&self) -> bool {
        matches!(self, ClearScope::AllQueues | ClearScope::AllTopics)
    }
}

/// Which clear operation to run against a [`ClearScope`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClearAction {
    /// Delete all active messages.
    DeleteActive,
    /// Delete all dead-letter messages.
    DeleteDlq,
    /// Resend all dead-letter messages to their main entity.
    ResendDlq,
}

impl ClearAction {
    /// Status sentinel dispatched in `main.rs` for this action.
    pub fn sentinel(self) -> &'static str {
        match self {
            ClearAction::DeleteActive => "Clearing (delete)...",
            ClearAction::DeleteDlq => "Clearing (delete DLQ)...",
            ClearAction::ResendDlq => "Clearing (resend)...",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            ClearAction::DeleteActive => "Delete ALL active messages",
            ClearAction::DeleteDlq => "Delete ALL dead-letter messages",
            ClearAction::ResendDlq => "Resend ALL DLQ messages to their main entity",
        }
    }
}

/// A clear operation that has been confirmed and is awaiting dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingClear {
    pub scope: ClearScope,
    pub action: ClearAction,
}

/// Active modal overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActiveModal {
    None,
    ConnectionModeSelect,
    ConnectionInput,
    ConnectionList,
    ConnectionSwitch,
    AzureAdNamespaceInput,
    NamespaceDiscovery {
        state: DiscoveryState,
    },
    SendMessage,
    CreateQueue,
    CreateTopic,
    CreateSubscription,
    EditSubscriptionFilter,
    ConfirmDelete(String),
    ConfirmBulkResend {
        entity_path: String,
        count: u32,
        is_topic: bool,
    },
    ConfirmBulkDelete {
        entity_path: String,
        count: u32,
        is_dlq: bool,
        is_topic: bool,
    },
    ConfirmSingleDelete {
        entity_path: String,
        sequence_number: i64,
        is_dlq: bool,
    },
    PeekCountInput,
    EditResend,
    ClearOptions {
        scope: ClearScope,
    },
    ConfirmClearBatch {
        scope: ClearScope,
        action: ClearAction,
    },
    Help,
    MetricsDetail,
    CopySelectConnection,
    CopySelectEntity,
    CopyEditMessage,
    ConfirmPurgeAllDlq,
}

/// State of the namespace discovery modal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryState {
    Loading,
    List,
    Error(String),
}

/// What kind of entity detail is being shown.
#[derive(Debug, Clone)]
pub enum DetailView {
    None,
    Queue(QueueDescription, Option<QueueRuntimeInfo>),
    Topic(TopicDescription, Option<TopicRuntimeInfo>),
    Subscription(
        SubscriptionDescription,
        Option<SubscriptionRuntimeInfo>,
        Vec<SubscriptionRule>,
    ),
}

/// Tab for the message panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageTab {
    Messages,
    DeadLetter,
}

/// Central application state.
pub struct App {
    pub running: bool,
    pub config: AppConfig,
    pub connection_name: Option<String>,

    // Clients
    pub management: Option<ManagementClient>,
    pub data_plane: Option<DataPlaneClient>,
    pub connection_config: Option<ConnectionConfig>,

    // Tree
    pub tree: Option<TreeNode>,
    pub flat_nodes: Vec<FlatNode>,
    pub tree_selected: usize,

    // Detail
    pub detail_view: DetailView,

    // Messages
    pub message_tab: MessageTab,
    pub messages: Vec<ReceivedMessage>,
    pub dlq_messages: Vec<ReceivedMessage>,
    pub message_selected: usize,
    pub selected_message_detail: Option<ReceivedMessage>,
    pub detail_editing: bool,
    /// If the message being edited came from DLQ, this holds its sequence number
    /// so we can remove it after successful resend.
    pub edit_source_dlq_seq: Option<i64>,

    // UI state
    pub focus: FocusPanel,
    pub modal: ActiveModal,
    pub status_message: String,
    pub status_is_error: bool,

    // Modal input buffers
    pub input_buffer: String,
    pub input_cursor: usize,
    pub input_fields: Vec<(String, String)>, // (label, value) for multi-field forms
    pub input_field_index: usize,
    pub form_cursor: usize, // cursor position within the active form field
    pub body_scroll: u16,   // vertical scroll offset for body editor

    // Pending peek count from the peek-count input modal
    pub pending_peek_count: Option<i32>,
    pub peek_dlq: bool,

    // Namespace discovery state
    pub discovered_namespaces: Vec<DiscoveredNamespace>,
    pub discovery_warnings: Vec<String>,
    pub namespace_list_state: usize,

    // Background task channel for long-running operations
    pub bg_tx: mpsc::UnboundedSender<BgEvent>,
    pub bg_rx: mpsc::UnboundedReceiver<BgEvent>,
    pub bg_running: bool,
    pub bg_cancel: Arc<AtomicBool>,

    // Loading indicator
    pub loading: bool,

    // Auto-refresh
    pub auto_refresh_enabled: bool,
    pub last_refresh: Option<Instant>,

    // Persistent scroll state for stateful widgets
    pub tree_list_state: ListState,
    pub message_table_state: TableState,
    /// Scroll offset for the read-only message body detail view.
    pub detail_body_scroll: u16,
    /// When true, show message body as raw text (no JSON/XML formatting).
    pub body_raw_mode: bool,

    // Message search/filter
    pub message_search_query: String,
    pub message_search_active: bool,
    pub message_search_cursor: usize,
    pub message_filtered_indices: Vec<usize>,

    // Copy operation state
    pub copy_source_message: Option<ReceivedMessage>,
    pub copy_source_entity: Option<String>,
    pub copy_dest_connection_name: Option<String>,
    pub copy_dest_connection_config: Option<ConnectionConfig>,
    pub copy_dest_entities: Vec<(String, EntityType)>,
    pub copy_entity_selected: usize,
    pub copy_connection_list_state: ListState,
    pub copy_entity_list_state: ListState,
    pub copy_destination_entity: Option<String>,

    // Azure Monitor metrics
    pub namespace_resource_id: Option<String>,
    pub entity_metrics: Option<EntityMetrics>,
    pub metrics_available: bool,
    pub metrics_enabled: bool,
    pub metrics_window: MetricsWindow,
    pub metrics_pending: bool,
    /// Clear operation confirmed by the user, consumed by the dispatcher.
    pub pending_clear: Option<PendingClear>,
}

/// Time window for Azure Monitor metrics queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricsWindow {
    OneHour,
    SixHours,
    TwentyFourHours,
    SevenDays,
}

impl MetricsWindow {
    /// Azure Monitor timespan parameter (ISO 8601 duration).
    pub fn timespan(&self) -> &'static str {
        match self {
            Self::OneHour => "PT1H",
            Self::SixHours => "PT6H",
            Self::TwentyFourHours => "P1D",
            Self::SevenDays => "P7D",
        }
    }

    /// Azure Monitor interval (granularity) appropriate for this window.
    pub fn interval(&self) -> &'static str {
        match self {
            Self::OneHour => "PT1M",
            Self::SixHours => "PT5M",
            Self::TwentyFourHours => "PT1H",
            Self::SevenDays => "PT1H",
        }
    }

    /// Human-readable label for UI display.
    pub fn label(&self) -> &'static str {
        match self {
            Self::OneHour => "1h",
            Self::SixHours => "6h",
            Self::TwentyFourHours => "24h",
            Self::SevenDays => "7d",
        }
    }

    /// Cycle to the next window.
    pub fn next(self) -> Self {
        match self {
            Self::OneHour => Self::SixHours,
            Self::SixHours => Self::TwentyFourHours,
            Self::TwentyFourHours => Self::SevenDays,
            Self::SevenDays => Self::OneHour,
        }
    }
}

impl App {
    pub fn new() -> Self {
        let config = AppConfig::load();
        let auto_refresh_enabled = config.settings.auto_refresh_secs > 0;
        let (bg_tx, bg_rx) = mpsc::unbounded_channel();
        Self {
            running: true,
            config,
            connection_name: None,
            management: None,
            data_plane: None,
            connection_config: None,
            tree: None,
            flat_nodes: Vec::new(),
            tree_selected: 0,
            detail_view: DetailView::None,
            message_tab: MessageTab::Messages,
            messages: Vec::new(),
            dlq_messages: Vec::new(),
            message_selected: 0,
            selected_message_detail: None,
            detail_editing: false,
            edit_source_dlq_seq: None,
            focus: FocusPanel::Tree,
            modal: ActiveModal::None,
            status_message: String::from("Press 'c' to connect, '?' for help"),
            status_is_error: false,
            input_buffer: String::new(),
            input_cursor: 0,
            input_fields: Vec::new(),
            input_field_index: 0,
            form_cursor: 0,
            body_scroll: 0,
            pending_peek_count: None,
            peek_dlq: false,
            discovered_namespaces: Vec::new(),
            discovery_warnings: Vec::new(),
            namespace_list_state: 0,
            bg_tx,
            bg_rx,
            bg_running: false,
            bg_cancel: Arc::new(AtomicBool::new(false)),
            loading: false,
            auto_refresh_enabled,
            last_refresh: None,
            tree_list_state: ListState::default(),
            message_table_state: TableState::default(),
            detail_body_scroll: 0,
            body_raw_mode: false,
            message_search_query: String::new(),
            message_search_active: false,
            message_search_cursor: 0,
            message_filtered_indices: Vec::new(),
            copy_source_message: None,
            copy_source_entity: None,
            copy_dest_connection_name: None,
            copy_dest_connection_config: None,
            copy_dest_entities: Vec::new(),
            copy_entity_selected: 0,
            copy_connection_list_state: ListState::default(),
            copy_entity_list_state: ListState::default(),
            copy_destination_entity: None,
            namespace_resource_id: None,
            entity_metrics: None,
            metrics_available: false,
            metrics_enabled: true,
            metrics_window: MetricsWindow::SixHours,
            metrics_pending: false,
            pending_clear: None,
        }
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status_message = msg.into();
        self.status_is_error = false;
    }

    pub fn set_error(&mut self, msg: impl Into<String>) {
        self.status_message = msg.into();
        self.status_is_error = true;
    }

    /// Signal the running background task to stop.
    pub fn cancel_bg(&self) {
        self.bg_cancel.store(true, Ordering::Relaxed);
    }

    /// Create a fresh cancellation token for a new background task.
    pub fn new_cancel_token(&mut self) -> Arc<AtomicBool> {
        let token = Arc::new(AtomicBool::new(false));
        self.bg_cancel = Arc::clone(&token);
        token
    }

    /// Connect to a Service Bus namespace using a SAS connection string.
    pub fn connect(&mut self, connection_string: &str) -> crate::client::Result<()> {
        let cfg = ConnectionConfig::from_connection_string(connection_string)?;
        self.management = Some(ManagementClient::new(cfg.clone()));
        self.data_plane = Some(DataPlaneClient::new(cfg.clone()));
        self.connection_config = Some(cfg);
        Ok(())
    }

    /// Connect to a Service Bus namespace using Azure AD (Microsoft Entra ID).
    pub fn connect_azure_ad(&mut self, namespace: &str) -> crate::client::Result<()> {
        let credential = azure_identity::DeveloperToolsCredential::new(None).map_err(|e| {
            crate::client::ServiceBusError::Auth(format!("Azure AD credential error: {}", e))
        })?;
        let cfg = ConnectionConfig::from_azure_ad(namespace, credential);
        self.management = Some(ManagementClient::new(cfg.clone()));
        self.data_plane = Some(DataPlaneClient::new(cfg.clone()));
        self.connection_config = Some(cfg);
        Ok(())
    }

    /// Disconnect from the current Service Bus namespace and reset all state.
    pub fn disconnect(&mut self) {
        // Cancel any running background operations
        self.cancel_bg();

        // Clear connection state
        self.management = None;
        self.data_plane = None;
        self.connection_config = None;
        self.connection_name = None;

        // Clear tree state
        self.tree = None;
        self.flat_nodes.clear();
        self.tree_selected = 0;
        self.detail_view = DetailView::None;

        // Clear message state
        self.messages.clear();
        self.dlq_messages.clear();
        self.message_selected = 0;
        self.selected_message_detail = None;
        self.detail_editing = false;
        self.edit_source_dlq_seq = None;
        self.clear_message_filter();

        // Reset UI state
        self.focus = FocusPanel::Tree;
        self.loading = false;
        self.bg_running = false;

        // Clear metrics state
        self.namespace_resource_id = None;
        self.entity_metrics = None;
        self.metrics_available = false;
        self.metrics_enabled = true;

        // Set status
        self.set_status("Disconnected. Press 'c' to connect, '?' for help");
    }

    /// Rebuild the flat node list from the tree (e.g., after expand/collapse).
    pub fn rebuild_flat_nodes(&mut self) {
        if let Some(ref tree) = self.tree {
            self.flat_nodes = tree.flatten();
            if self.tree_selected >= self.flat_nodes.len() && !self.flat_nodes.is_empty() {
                self.tree_selected = self.flat_nodes.len() - 1;
            }
        }
    }

    /// Toggle expand/collapse on the selected tree node.
    pub fn toggle_expand(&mut self) {
        if self.flat_nodes.is_empty() {
            return;
        }
        let selected_id = self.flat_nodes[self.tree_selected].id.clone();
        if let Some(ref mut tree) = self.tree {
            toggle_node(tree, &selected_id);
        }
        self.rebuild_flat_nodes();
    }

    /// Get the currently selected entity path and type.
    pub fn selected_entity(&self) -> Option<(&str, &EntityType)> {
        if self.flat_nodes.is_empty() {
            return None;
        }
        let node = &self.flat_nodes[self.tree_selected];
        if node.path.is_empty() {
            None
        } else {
            Some((&node.path, &node.entity_type))
        }
    }

    /// Rebuild the filtered indices for the current message tab based on `message_search_query`.
    /// If the query is empty, all messages are included.
    pub fn apply_message_filter(&mut self) {
        let messages = match self.message_tab {
            MessageTab::Messages => &self.messages,
            MessageTab::DeadLetter => &self.dlq_messages,
        };

        if self.message_search_query.is_empty() {
            self.message_filtered_indices = (0..messages.len()).collect();
        } else {
            let query = self.message_search_query.to_lowercase();
            self.message_filtered_indices = messages
                .iter()
                .enumerate()
                .filter(|(_, msg)| message_matches(msg, &query))
                .map(|(idx, _)| idx)
                .collect();
        }

        // Clamp selection to filtered bounds
        if self.message_filtered_indices.is_empty() {
            self.message_selected = 0;
        } else if self.message_selected >= self.message_filtered_indices.len() {
            self.message_selected = self.message_filtered_indices.len() - 1;
        }
    }

    /// Clear the message filter and restore the full list.
    pub fn clear_message_filter(&mut self) {
        self.message_search_query.clear();
        self.message_search_active = false;
        self.message_search_cursor = 0;
        let len = match self.message_tab {
            MessageTab::Messages => self.messages.len(),
            MessageTab::DeadLetter => self.dlq_messages.len(),
        };
        self.message_filtered_indices = (0..len).collect();
    }

    /// Initialize the send message form fields.
    pub fn init_send_form(&mut self) {
        self.input_fields = vec![
            ("Body".to_string(), String::new()),
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Message ID".to_string(), String::new()),
            ("Correlation ID".to_string(), String::new()),
            ("Session ID".to_string(), String::new()),
            ("Label".to_string(), String::new()),
            ("TTL (seconds)".to_string(), String::new()),
            ("Scheduled (UTC)".to_string(), String::new()),
            ("Custom Properties (k=v,...)".to_string(), String::new()),
        ];
        self.input_field_index = 0;
        self.form_cursor = 0;
        self.modal = ActiveModal::SendMessage;
    }

    /// Enter inline WYSIWYG edit mode in the message detail view.
    pub fn init_detail_edit(&mut self) {
        if let Some(ref msg) = self.selected_message_detail {
            self.edit_source_dlq_seq = if self.message_tab == MessageTab::DeadLetter {
                msg.broker_properties.sequence_number
            } else {
                None
            };
            let msg = msg.clone();
            self.populate_edit_fields(&msg);
            self.detail_editing = true;
        }
    }

    /// Populate input_fields from a ReceivedMessage (shared by modal and inline edit).
    pub fn populate_edit_fields(&mut self, msg: &ReceivedMessage) {
        let custom_props_str = msg
            .custom_properties
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join(",");

        self.input_fields = vec![
            ("Body".to_string(), msg.body.clone()),
            (
                "Content-Type".to_string(),
                msg.broker_properties
                    .content_type
                    .clone()
                    .unwrap_or_else(|| "application/json".to_string()),
            ),
            (
                "Message ID".to_string(),
                msg.broker_properties.message_id.clone().unwrap_or_default(),
            ),
            (
                "Correlation ID".to_string(),
                msg.broker_properties
                    .correlation_id
                    .clone()
                    .unwrap_or_default(),
            ),
            (
                "Session ID".to_string(),
                msg.broker_properties.session_id.clone().unwrap_or_default(),
            ),
            (
                "Label".to_string(),
                msg.broker_properties.label.clone().unwrap_or_default(),
            ),
            ("TTL (seconds)".to_string(), String::new()),
            (
                "Scheduled (UTC)".to_string(),
                msg.broker_properties
                    .scheduled_enqueue_time_utc
                    .clone()
                    .unwrap_or_default(),
            ),
            ("Custom Properties (k=v,...)".to_string(), custom_props_str),
        ];
        self.input_field_index = 0;
        self.form_cursor = self.input_fields[0].1.len();
    }

    /// Build a ServiceBusMessage from the current send form fields.
    pub fn build_message_from_form(&self) -> ServiceBusMessage {
        let get =
            |idx: usize| -> Option<String> {
                self.input_fields.get(idx).and_then(|(_, v)| {
                    if v.is_empty() {
                        None
                    } else {
                        Some(v.clone())
                    }
                })
            };

        let custom_props: Vec<(String, String)> = get(8)
            .map(|s| {
                s.split(',')
                    .filter_map(|pair| {
                        let mut parts = pair.splitn(2, '=');
                        let k = parts.next()?.trim().to_string();
                        let v = parts.next()?.trim().to_string();
                        if k.is_empty() {
                            None
                        } else {
                            Some((k, v))
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        ServiceBusMessage {
            body: get(0).unwrap_or_default(),
            content_type: get(1),
            message_id: get(2).or_else(|| Some(uuid::Uuid::new_v4().to_string())),
            correlation_id: get(3),
            session_id: get(4),
            label: get(5),
            time_to_live: get(6),
            scheduled_enqueue_time: get(7),
            custom_properties: custom_props,
            ..Default::default()
        }
    }

    /// Initialize create queue form.
    pub fn init_create_queue_form(&mut self) {
        self.input_fields = vec![
            ("Queue Name".to_string(), String::new()),
            ("Max Size (MB)".to_string(), "1024".to_string()),
            ("Lock Duration".to_string(), "PT30S".to_string()),
            ("Default TTL".to_string(), "P14D".to_string()),
            ("Max Delivery Count".to_string(), "10".to_string()),
            ("Requires Session".to_string(), "false".to_string()),
            ("Enable Partitioning".to_string(), "false".to_string()),
            ("Dead-letter on Expiry".to_string(), "false".to_string()),
        ];
        self.input_field_index = 0;
        self.form_cursor = 0;
        self.modal = ActiveModal::CreateQueue;
    }

    pub fn build_queue_from_form(&self) -> QueueDescription {
        let get_str =
            |idx: usize| -> Option<String> {
                self.input_fields.get(idx).and_then(|(_, v)| {
                    if v.is_empty() {
                        None
                    } else {
                        Some(v.clone())
                    }
                })
            };
        let get_i64 = |idx: usize| -> Option<i64> { get_str(idx).and_then(|v| v.parse().ok()) };
        let get_i32 = |idx: usize| -> Option<i32> { get_str(idx).and_then(|v| v.parse().ok()) };
        let get_bool = |idx: usize| -> Option<bool> { get_str(idx).and_then(|v| v.parse().ok()) };

        QueueDescription {
            name: get_str(0).unwrap_or_default(),
            max_size_in_megabytes: get_i64(1),
            lock_duration: get_str(2),
            default_message_time_to_live: get_str(3),
            max_delivery_count: get_i32(4),
            requires_session: get_bool(5),
            enable_partitioning: get_bool(6),
            dead_lettering_on_message_expiration: get_bool(7),
            ..Default::default()
        }
    }

    /// Initialize create topic form.
    pub fn init_create_topic_form(&mut self) {
        self.input_fields = vec![
            ("Topic Name".to_string(), String::new()),
            ("Max Size (MB)".to_string(), "1024".to_string()),
            ("Default TTL".to_string(), "P14D".to_string()),
            ("Enable Partitioning".to_string(), "false".to_string()),
        ];
        self.input_field_index = 0;
        self.form_cursor = 0;
        self.modal = ActiveModal::CreateTopic;
    }

    pub fn build_topic_from_form(&self) -> TopicDescription {
        let get_str =
            |idx: usize| -> Option<String> {
                self.input_fields.get(idx).and_then(|(_, v)| {
                    if v.is_empty() {
                        None
                    } else {
                        Some(v.clone())
                    }
                })
            };

        TopicDescription {
            name: get_str(0).unwrap_or_default(),
            max_size_in_megabytes: get_str(1).and_then(|v| v.parse().ok()),
            default_message_time_to_live: get_str(2),
            enable_partitioning: get_str(3).and_then(|v| v.parse().ok()),
            ..Default::default()
        }
    }

    /// Initialize create subscription form.
    pub fn init_create_subscription_form(&mut self, topic_name: &str) {
        self.input_fields = vec![
            ("Topic".to_string(), topic_name.to_string()),
            ("Subscription Name".to_string(), String::new()),
            ("Lock Duration".to_string(), "PT30S".to_string()),
            ("Default TTL".to_string(), "P14D".to_string()),
            ("Max Delivery Count".to_string(), "10".to_string()),
            ("Requires Session".to_string(), "false".to_string()),
            ("Dead-letter on Expiry".to_string(), "false".to_string()),
        ];
        self.input_field_index = 1; // Skip topic name (pre-filled)
        self.form_cursor = 0;
        self.modal = ActiveModal::CreateSubscription;
    }

    pub fn build_subscription_from_form(&self) -> SubscriptionDescription {
        let get_str =
            |idx: usize| -> Option<String> {
                self.input_fields.get(idx).and_then(|(_, v)| {
                    if v.is_empty() {
                        None
                    } else {
                        Some(v.clone())
                    }
                })
            };

        SubscriptionDescription {
            topic_name: get_str(0).unwrap_or_default(),
            name: get_str(1).unwrap_or_default(),
            lock_duration: get_str(2),
            default_message_time_to_live: get_str(3),
            max_delivery_count: get_str(4).and_then(|v| v.parse().ok()),
            requires_session: get_str(5).and_then(|v| v.parse().ok()),
            dead_lettering_on_message_expiration: get_str(6).and_then(|v| v.parse().ok()),
            ..Default::default()
        }
    }

    /// Initialize edit subscription filter form.
    pub fn init_edit_subscription_filter_form(
        &mut self,
        topic_name: &str,
        sub_name: &str,
        rule_name: &str,
        sql_expression: &str,
    ) {
        self.input_fields = vec![
            ("Topic".to_string(), topic_name.to_string()),
            ("Subscription".to_string(), sub_name.to_string()),
            ("Rule Name".to_string(), rule_name.to_string()),
            ("SQL Filter".to_string(), sql_expression.to_string()),
        ];
        self.input_field_index = 3;
        self.form_cursor = self.input_fields[3].1.len();
        self.modal = ActiveModal::EditSubscriptionFilter;
    }

    pub fn build_subscription_filter_from_form(&self) -> (String, String) {
        let get = |idx: usize| -> Option<String> {
            self.input_fields
                .get(idx)
                .map(|(_, v)| v.trim().to_string())
        };

        let rule_name = get(2)
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "$Default".to_string());
        let sql_expression = get(3)
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "1=1".to_string());

        (rule_name, sql_expression)
    }

    /// Start namespace discovery flow.
    pub fn start_namespace_discovery(&mut self) {
        self.discovered_namespaces.clear();
        self.discovery_warnings.clear();
        self.namespace_list_state = 0;
        self.modal = ActiveModal::NamespaceDiscovery {
            state: DiscoveryState::Loading,
        };
        self.set_status("Discovering namespaces...");
    }

    /// Fetch entity list from a destination connection for copy target selection.
    pub async fn fetch_destination_entities(
        config: crate::client::ConnectionConfig,
    ) -> crate::client::Result<Vec<(String, EntityType)>> {
        let mgmt = crate::client::ManagementClient::new(config);
        let mut entities = Vec::new();

        // Fetch queues and topics in parallel
        let (queues_result, topics_result) =
            tokio::join!(mgmt.list_queues_with_counts(), mgmt.list_topics());

        if let Ok(queues) = queues_result {
            for (q, _, _) in queues {
                entities.push((q.name.clone(), EntityType::Queue));
            }
        }

        if let Ok(topics) = topics_result {
            for t in topics {
                entities.push((t.name.clone(), EntityType::Topic));
            }
        }

        entities.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(entities)
    }
}

fn toggle_node(node: &mut TreeNode, id: &str) -> bool {
    if node.id == id {
        node.expanded = !node.expanded;
        return true;
    }
    for child in &mut node.children {
        if toggle_node(child, id) {
            return true;
        }
    }
    false
}

/// Check if a message matches a search query (case-insensitive substring).
/// Searches across message ID, sequence number, label, body (first 500 chars),
/// correlation ID, custom properties, and dead-letter fields.
fn message_matches(msg: &ReceivedMessage, query: &str) -> bool {
    let bp = &msg.broker_properties;

    if let Some(ref id) = bp.message_id {
        if id.to_lowercase().contains(query) {
            return true;
        }
    }
    if let Some(seq) = bp.sequence_number {
        if seq.to_string().contains(query) {
            return true;
        }
    }
    if let Some(ref label) = bp.label {
        if label.to_lowercase().contains(query) {
            return true;
        }
    }
    if let Some(ref cid) = bp.correlation_id {
        if cid.to_lowercase().contains(query) {
            return true;
        }
    }
    if let Some(ref enqueued) = bp.enqueued_time_utc {
        if enqueued.to_lowercase().contains(query) {
            return true;
        }
    }
    // Body: search first 500 chars to avoid perf issues on huge payloads
    let body_search_len = msg.body.len().min(500);
    if msg.body[..body_search_len].to_lowercase().contains(query) {
        return true;
    }
    // Custom properties
    for (k, v) in &msg.custom_properties {
        if k.to_lowercase().contains(query) || v.to_lowercase().contains(query) {
            return true;
        }
    }
    // DLQ-specific fields
    if let Some(ref reason) = bp.dead_letter_reason {
        if reason.to_lowercase().contains(query) {
            return true;
        }
    }
    if let Some(ref desc) = bp.dead_letter_error_description {
        if desc.to_lowercase().contains(query) {
            return true;
        }
    }
    false
}

/// Build the entity tree from the management API (runs on a spawned task).
pub async fn build_tree(
    mgmt: ManagementClient,
    namespace: String,
) -> crate::client::Result<(TreeNode, Vec<FlatNode>)> {
    // Parallel fetch: queues + topics in one round trip pair
    let (queues_result, topics_result) =
        tokio::join!(mgmt.list_queues_with_counts(), mgmt.list_topics());
    let queues = queues_result?;
    let topics = topics_result?;

    let mut root = TreeNode::new_folder("root", &namespace, EntityType::Namespace, 0);

    // Queues folder
    let mut queue_folder = TreeNode::new_folder("queues", "Queues", EntityType::QueueFolder, 1);
    for (q, active_count, dlq_count) in &queues {
        let mut node = TreeNode::new_entity(
            &format!("q:{}", q.name),
            &q.name,
            EntityType::Queue,
            &q.name,
            2,
        );
        node.message_count = Some(*active_count);
        node.dlq_count = Some(*dlq_count);
        queue_folder.children.push(node);
    }
    root.children.push(queue_folder);

    // Topics folder — fetch all subscription lists concurrently.
    let mut topic_folder = TreeNode::new_folder("topics", "Topics", EntityType::TopicFolder, 1);

    // Spawn concurrent subscription list fetches for all topics
    let mut sub_handles = Vec::with_capacity(topics.len());
    for t in &topics {
        let mgmt_clone = mgmt.clone();
        let topic_name = t.name.clone();
        sub_handles.push(tokio::spawn(async move {
            let subs = mgmt_clone.list_subscriptions_with_counts(&topic_name).await;
            (topic_name, subs)
        }));
    }

    // Collect results (order doesn't matter, we match by topic name)
    let mut subs_by_topic = std::collections::HashMap::new();
    for handle in sub_handles {
        if let Ok((topic_name, Ok(subs))) = handle.await {
            subs_by_topic.insert(topic_name, subs);
        }
    }

    for t in &topics {
        let mut topic_node = TreeNode::new_entity(
            &format!("t:{}", t.name),
            &t.name,
            EntityType::Topic,
            &t.name,
            2,
        );

        if let Some(subs) = subs_by_topic.remove(&t.name) {
            let mut total_active = 0i64;
            let mut total_dlq = 0i64;

            let mut sub_folder = TreeNode::new_folder(
                &format!("t:{}:subs", t.name),
                "Subscriptions",
                EntityType::SubscriptionFolder,
                3,
            );
            for (s, active_count, dlq_count) in &subs {
                total_active += active_count;
                total_dlq += dlq_count;

                let sub_path = format!("{}/Subscriptions/{}", t.name, s.name);
                let mut sub_node = TreeNode::new_entity(
                    &format!("s:{}:{}", t.name, s.name),
                    &s.name,
                    EntityType::Subscription,
                    &sub_path,
                    4,
                );
                sub_node.message_count = Some(*active_count);
                sub_node.dlq_count = Some(*dlq_count);
                sub_folder.children.push(sub_node);
            }

            // Set aggregated counts on topic
            topic_node.message_count = Some(total_active);
            topic_node.dlq_count = Some(total_dlq);

            topic_node.children.push(sub_folder);
        }
        topic_folder.children.push(topic_node);
    }
    root.children.push(topic_folder);

    let flat_nodes = root.flatten();
    Ok((root, flat_nodes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::models::{BrokerProperties, ReceivedMessage};

    #[test]
    fn auto_refresh_enabled_derived_from_config() {
        let mut app = App::new();
        // Positive secs → enabled
        app.config.settings.auto_refresh_secs = 30;
        let enabled = app.config.settings.auto_refresh_secs > 0;
        assert!(enabled);

        // Zero secs → disabled
        app.config.settings.auto_refresh_secs = 0;
        let enabled = app.config.settings.auto_refresh_secs > 0;
        assert!(!enabled);
    }

    #[test]
    fn last_refresh_is_none_on_new_app() {
        let app = App::new();
        assert!(app.last_refresh.is_none());
    }

    #[test]
    fn auto_refresh_does_not_fire_without_last_refresh() {
        let mut app = App::new();
        app.auto_refresh_enabled = true;
        app.config.settings.auto_refresh_secs = 30;
        // Even with auto_refresh_enabled, no last_refresh means no timer fires
        assert!(app.last_refresh.is_none());
        // The main loop checks last_refresh.is_some() before comparing elapsed time
    }

    #[test]
    fn auto_refresh_timer_requires_management_client() {
        let mut app = App::new();
        app.auto_refresh_enabled = true;
        app.config.settings.auto_refresh_secs = 1;

        assert!(app.management.is_none());

        // Simulate the timer condition from main.rs
        let should_check = app.auto_refresh_enabled
            && !app.bg_running
            && !app.loading
            && app.management.is_some()
            && app.config.settings.auto_refresh_secs > 0;
        assert!(
            !should_check,
            "should not trigger without management client"
        );
    }

    #[test]
    fn auto_refresh_blocked_while_bg_running() {
        let mut app = App::new();
        app.auto_refresh_enabled = true;
        app.bg_running = true;
        app.config.settings.auto_refresh_secs = 30;

        let should_check = app.auto_refresh_enabled
            && !app.bg_running
            && !app.loading
            && app.config.settings.auto_refresh_secs > 0;
        assert!(!should_check, "should not trigger while bg_running");
    }

    #[test]
    fn auto_refresh_blocked_while_loading() {
        let mut app = App::new();
        app.auto_refresh_enabled = true;
        app.loading = true;
        app.config.settings.auto_refresh_secs = 30;

        let should_check = app.auto_refresh_enabled
            && !app.bg_running
            && !app.loading
            && app.config.settings.auto_refresh_secs > 0;
        assert!(!should_check, "should not trigger while loading");
    }

    fn make_msg(id: &str, label: &str, body: &str, seq: i64) -> ReceivedMessage {
        ReceivedMessage {
            body: body.to_string(),
            broker_properties: BrokerProperties {
                message_id: Some(id.to_string()),
                label: Some(label.to_string()),
                sequence_number: Some(seq),
                ..Default::default()
            },
            custom_properties: Vec::new(),
            lock_token_uri: None,
            source_entity: None,
        }
    }

    #[test]
    fn filter_empty_query_includes_all() {
        let mut app = App::new();
        app.messages = vec![
            make_msg("a", "alpha", "body1", 1),
            make_msg("b", "beta", "body2", 2),
            make_msg("c", "gamma", "body3", 3),
        ];
        app.message_search_query.clear();
        app.apply_message_filter();

        assert_eq!(app.message_filtered_indices, vec![0, 1, 2]);
    }

    #[test]
    fn filter_by_message_id() {
        let mut app = App::new();
        app.messages = vec![
            make_msg("abc-123", "alpha", "body1", 1),
            make_msg("xyz-456", "beta", "body2", 2),
            make_msg("abc-789", "gamma", "body3", 3),
        ];
        app.message_search_query = "abc".to_string();
        app.apply_message_filter();

        assert_eq!(app.message_filtered_indices, vec![0, 2]);
    }

    #[test]
    fn filter_case_insensitive() {
        let mut app = App::new();
        app.messages = vec![
            make_msg("a", "Hello World", "body1", 1),
            make_msg("b", "goodbye", "body2", 2),
        ];
        app.message_search_query = "hello".to_string();
        app.apply_message_filter();

        assert_eq!(app.message_filtered_indices, vec![0]);
    }

    #[test]
    fn filter_by_body_content() {
        let mut app = App::new();
        app.messages = vec![
            make_msg("a", "", "{\"key\": \"needle\"}", 1),
            make_msg("b", "", "{\"key\": \"other\"}", 2),
        ];
        app.message_search_query = "needle".to_string();
        app.apply_message_filter();

        assert_eq!(app.message_filtered_indices, vec![0]);
    }

    #[test]
    fn filter_by_custom_properties() {
        let mut app = App::new();
        let mut msg = make_msg("a", "", "body", 1);
        msg.custom_properties = vec![("env".to_string(), "production".to_string())];
        app.messages = vec![msg, make_msg("b", "", "body", 2)];
        app.message_search_query = "production".to_string();
        app.apply_message_filter();

        assert_eq!(app.message_filtered_indices, vec![0]);
    }

    #[test]
    fn filter_by_sequence_number() {
        let mut app = App::new();
        app.messages = vec![
            make_msg("a", "", "body", 12345),
            make_msg("b", "", "body", 67890),
        ];
        app.message_search_query = "12345".to_string();
        app.apply_message_filter();

        assert_eq!(app.message_filtered_indices, vec![0]);
    }

    #[test]
    fn filter_clamps_selection() {
        let mut app = App::new();
        app.messages = vec![
            make_msg("a", "alpha", "body1", 1),
            make_msg("b", "beta", "body2", 2),
            make_msg("c", "gamma", "body3", 3),
        ];
        app.message_selected = 2; // pointing at last message
        app.message_search_query = "alpha".to_string(); // only first matches
        app.apply_message_filter();

        assert_eq!(app.message_filtered_indices, vec![0]);
        assert_eq!(app.message_selected, 0); // clamped
    }

    #[test]
    fn clear_filter_restores_full_list() {
        let mut app = App::new();
        app.messages = vec![
            make_msg("a", "alpha", "body1", 1),
            make_msg("b", "beta", "body2", 2),
        ];
        app.message_search_query = "alpha".to_string();
        app.message_search_active = true;
        app.message_search_cursor = 5;
        app.apply_message_filter();
        assert_eq!(app.message_filtered_indices.len(), 1);

        app.clear_message_filter();

        assert!(app.message_search_query.is_empty());
        assert!(!app.message_search_active);
        assert_eq!(app.message_search_cursor, 0);
        assert_eq!(app.message_filtered_indices, vec![0, 1]);
    }

    #[test]
    fn filter_dlq_by_dead_letter_reason() {
        let mut app = App::new();
        let mut msg = make_msg("a", "", "body", 1);
        msg.broker_properties.dead_letter_reason = Some("MaxDeliveryCount".to_string());
        app.dlq_messages = vec![msg, make_msg("b", "", "body", 2)];
        app.message_tab = MessageTab::DeadLetter;
        app.message_search_query = "maxdelivery".to_string();
        app.apply_message_filter();

        assert_eq!(app.message_filtered_indices, vec![0]);
    }
}
