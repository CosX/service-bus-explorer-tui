use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use ratatui::widgets::{ListState, TableState};
use tokio::sync::mpsc;

use crate::client::models::*;
use crate::client::{ConnectionConfig, DataPlaneClient, ManagementClient};
use crate::config::AppConfig;

/// Events sent from background tasks back to the main loop.
pub enum BgEvent {
    Progress(String),
    PurgeComplete { count: u64 },
    ResendComplete { resent: u32, errors: u32 },
    BulkDeleteComplete { deleted: u32, was_dlq: bool },
    Cancelled { message: String },
    Failed(String),

    // Non-blocking async operation results
    TreeRefreshed {
        tree: TreeNode,
        flat_nodes: Vec<FlatNode>,
    },
    DetailLoaded(DetailView),
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
}

/// Which panel is currently focused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusPanel {
    Tree,
    Detail,
    Messages,
}

/// Active modal overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActiveModal {
    None,
    ConnectionModeSelect,
    ConnectionInput,
    ConnectionList,
    AzureAdNamespaceInput,
    SendMessage,
    CreateQueue,
    CreateTopic,
    CreateSubscription,
    ConfirmDelete(String),
    ConfirmBulkResend { entity_path: String, count: u32, is_topic: bool },
    ConfirmBulkDelete { entity_path: String, count: u32, is_dlq: bool, is_topic: bool },
    PeekCountInput,
    EditResend,
    ClearOptions {
        entity_path: String,
        base_entity_path: String,
        is_topic: bool,
    },
    Help,
}

/// What kind of entity detail is being shown.
#[derive(Debug, Clone)]
pub enum DetailView {
    None,
    Queue(QueueDescription, Option<QueueRuntimeInfo>),
    Topic(TopicDescription, Option<TopicRuntimeInfo>),
    Subscription(SubscriptionDescription, Option<SubscriptionRuntimeInfo>),
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
    pub tree_filter: String,

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

    // Background task channel for long-running operations
    pub bg_tx: mpsc::UnboundedSender<BgEvent>,
    pub bg_rx: mpsc::UnboundedReceiver<BgEvent>,
    pub bg_running: bool,
    pub bg_cancel: Arc<AtomicBool>,

    // Loading indicator
    pub loading: bool,

    // Persistent scroll state for stateful widgets
    pub tree_list_state: ListState,
    pub message_table_state: TableState,
    /// Scroll offset for the read-only message body detail view.
    pub detail_body_scroll: u16,
}

impl App {
    pub fn new() -> Self {
        let config = AppConfig::load();
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
            tree_filter: String::new(),
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
            bg_tx,
            bg_rx,
            bg_running: false,
            bg_cancel: Arc::new(AtomicBool::new(false)),
            loading: false,
            tree_list_state: ListState::default(),
            message_table_state: TableState::default(),
            detail_body_scroll: 0,
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
        let credential = azure_identity::DefaultAzureCredential::new()
            .map_err(|e| crate::client::ServiceBusError::Auth(format!("Azure AD credential error: {}", e)))?;
        let cfg = ConnectionConfig::from_azure_ad(namespace, credential);
        self.management = Some(ManagementClient::new(cfg.clone()));
        self.data_plane = Some(DataPlaneClient::new(cfg.clone()));
        self.connection_config = Some(cfg);
        Ok(())
    }

    /// Refresh the entity tree from the management API.
    ///
    /// Fetches entity lists only (queues, topics, subscriptions). Runtime info
    /// (message counts) is loaded lazily when a node is selected in the detail
    /// panel — this avoids an O(N) sequential HTTP round trip per entity.
    pub async fn refresh_tree(&mut self) -> crate::client::Result<()> {
        let mgmt = self
            .management
            .as_ref()
            .ok_or_else(|| crate::client::ServiceBusError::Operation("Not connected".into()))?
            .clone();

        let namespace = self
            .connection_config
            .as_ref()
            .map(|c| c.namespace.clone())
            .unwrap_or_else(|| "Namespace".to_string());

        let (tree, flat_nodes) = build_tree(mgmt, namespace).await?;
        self.flat_nodes = flat_nodes;
        self.tree = Some(tree);
        if self.tree_selected >= self.flat_nodes.len() {
            self.tree_selected = 0;
        }

        Ok(())
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
            ("Custom Properties (k=v,...)".to_string(), String::new()),
        ];
        self.input_field_index = 0;
        self.form_cursor = 0;
        self.modal = ActiveModal::SendMessage;
    }

    /// Initialize the edit & resend form, pre-populated from a received message.
    pub fn init_edit_resend_form(&mut self, msg: &ReceivedMessage) {
        self.edit_source_dlq_seq = if self.message_tab == MessageTab::DeadLetter {
            msg.broker_properties.sequence_number
        } else {
            None
        };
        self.populate_edit_fields(msg);
        self.modal = ActiveModal::EditResend;
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
    fn populate_edit_fields(&mut self, msg: &ReceivedMessage) {
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
                "Custom Properties (k=v,...)".to_string(),
                custom_props_str,
            ),
        ];
        self.input_field_index = 0;
        self.form_cursor = self.input_fields[0].1.len();
    }

    /// Build a ServiceBusMessage from the current send form fields.
    pub fn build_message_from_form(&self) -> ServiceBusMessage {
        let get = |idx: usize| -> Option<String> {
            self.input_fields
                .get(idx)
                .and_then(|(_, v)| {
                    if v.is_empty() {
                        None
                    } else {
                        Some(v.clone())
                    }
                })
        };

        let custom_props: Vec<(String, String)> = get(7)
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
        let get_str = |idx: usize| -> Option<String> {
            self.input_fields.get(idx).and_then(|(_, v)| {
                if v.is_empty() { None } else { Some(v.clone()) }
            })
        };
        let get_i64 = |idx: usize| -> Option<i64> {
            get_str(idx).and_then(|v| v.parse().ok())
        };
        let get_i32 = |idx: usize| -> Option<i32> {
            get_str(idx).and_then(|v| v.parse().ok())
        };
        let get_bool = |idx: usize| -> Option<bool> {
            get_str(idx).and_then(|v| v.parse().ok())
        };

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
        let get_str = |idx: usize| -> Option<String> {
            self.input_fields.get(idx).and_then(|(_, v)| {
                if v.is_empty() { None } else { Some(v.clone()) }
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
        let get_str = |idx: usize| -> Option<String> {
            self.input_fields.get(idx).and_then(|(_, v)| {
                if v.is_empty() { None } else { Some(v.clone()) }
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

/// Build the entity tree from the management API (runs on a spawned task).
pub async fn build_tree(
    mgmt: ManagementClient,
    namespace: String,
) -> crate::client::Result<(TreeNode, Vec<FlatNode>)> {
    // Parallel fetch: queues + topics in one round trip pair
    let (queues_result, topics_result) = tokio::join!(
        mgmt.list_queues_with_counts(),
        mgmt.list_topics()
    );
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
            let mut sub_folder = TreeNode::new_folder(
                &format!("t:{}:subs", t.name),
                "Subscriptions",
                EntityType::SubscriptionFolder,
                3,
            );
            for (s, active_count, dlq_count) in &subs {
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
            topic_node.children.push(sub_folder);
        }
        topic_folder.children.push(topic_node);
    }
    root.children.push(topic_folder);

    let flat_nodes = root.flatten();
    Ok((root, flat_nodes))
}
