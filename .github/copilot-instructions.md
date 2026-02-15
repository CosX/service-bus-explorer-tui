# Service Bus Explorer TUI — Copilot Instructions

## Project Overview

A cross-platform Rust TUI for managing Azure Service Bus (queues, topics, subscriptions, messages). Built with `ratatui` and direct REST API integration — **no official Azure SDK**.

## Architecture

### No Azure SDK
All client code in `src/client/` uses `reqwest` against the REST API directly. Management plane uses ATOM XML feeds parsed with targeted string extraction (not full serde XML). Data plane uses JSON/HTTP. Auth is HMAC-SHA256 SAS tokens or Azure AD Bearer tokens via `azure_identity`.

### Async + Sync Hybrid Event Loop
The main loop in `main.rs` polls `crossterm` events synchronously at 100ms intervals. Azure operations are dispatched as `tokio::spawn` tasks that communicate results back via an unbounded `mpsc` channel (`app.bg_tx` / `app.bg_rx`). The main loop drains `bg_rx.try_recv()` each tick.

### Status Sentinel Dispatch
Action dispatch in `run_app()` uses **string matching on `app.status_message`** to trigger async operations. `event.rs` sets the sentinel; `main.rs` matches it and spawns the task. Known sentinels:
- `"Peeking messages..."` → peek, `"Refreshing..."` → tree reload
- `"Submitting..."` → send/create/edit (disambiguated by `app.modal` or `app.detail_editing`)
- `"Deleting..."` → entity delete, `"Bulk resending..."` / `"Bulk deleting..."` → bulk ops
- `"Clearing (delete)..."` / `"Clearing (delete DLQ)..."` / `"Clearing (resend)..."` → purge/resend

When adding new operations, set a unique sentinel in `event.rs`, then match it in `main.rs`.

### ATOM XML Parsing
Azure returns inconsistent ATOM feed schemas. Parsing in `management.rs` uses raw string search (`extract_element`, `extract_element_value`, `extract_entries`) rather than serde XML deserialization. `CountDetails` child elements may be prefixed (`d2p1:ActiveMessageCount`) or unprefixed — the parser tries both.

## Module Structure

```
src/
├── main.rs              # Entry point, event loop, status-sentinel → async task dispatch
├── app.rs               # App state, BgEvent enum, form builders, tree construction
├── event.rs             # Input routing: global → modal → panel handlers
├── config.rs            # TOML persistence (connections, settings, OS-specific paths)
├── client/
│   ├── auth.rs          # SAS token gen, Azure AD token, connection string parsing
│   ├── management.rs    # Management plane: ATOM XML CRUD + raw XML parsing helpers
│   ├── data_plane.rs    # Data plane: send, peek-lock, receive-delete, purge, bulk ops
│   ├── models.rs        # Entity descriptions, message models, TreeNode/FlatNode
│   └── error.rs         # ServiceBusError (thiserror) with Api, Auth, Xml variants
└── ui/
    ├── layout.rs        # Top-level 3-panel layout (tree | detail | messages)
    ├── tree.rs          # Entity tree with inline message/DLQ counts
    ├── messages.rs      # Message list + detail view + inline edit rendering
    ├── modals.rs        # Connection, form, confirm, clear-options, peek-count dialogs
    ├── detail.rs        # Entity properties/runtime info panel
    ├── status_bar.rs    # Bottom status bar
    ├── help.rs          # Full keyboard shortcut overlay (`?` key)
    └── sanitize.rs      # Terminal escape injection prevention (CSI/OSC stripping)
```

## Key Patterns

### Entity Path Handling
- Queues: `"queue-name"` — Topics: `"topic-name"` — Subscriptions: `"topic-name/Subscriptions/sub-name"`
- Management API uses `/Subscriptions/` (PascalCase); data plane requires `/subscriptions/` (lowercase). `DataPlaneClient::normalize_path()` handles conversion.
- DLQ: append `/$deadletterqueue` to any entity path
- **Send path**: subscriptions must route to parent topic — use `send_path()` in `main.rs` to strip `/Subscriptions/...`

### Topic Fan-out
Operations on a **Topic** entity automatically fan out across all its subscriptions:
- Peek DLQ on topic → peeks all subscription DLQs and merges results
- Purge/clear on topic → purges each subscription (or each subscription's DLQ)
- Bulk resend on topic → resends from each subscription's DLQ to the topic

This pattern appears in every topic-aware spawn block in `main.rs`. It calls `mgmt.list_subscriptions()` first, then iterates.

### Peek Implementation
The REST API's `PeekOnly=true` has no cursor (always returns the same first message). Peek is implemented as **peek-lock + abandon**: lock N messages sequentially, collect them, then abandon all locks. This makes messages available again but **increments `DeliveryCount`** on each peek.

### Input Routing Priority
`event.rs:handle_events()` routes keys in this order:
1. **Background running + Esc** → cancel operation
2. **Modal open** (`app.modal != None`) → `handle_modal_input()`
3. **Inline editing** (`app.detail_editing`) → `handle_detail_edit_input()` (Esc exits edit, all other keys go to field editor)
4. **Global keys** (q, Ctrl+C, ?, c, Tab)
5. **Panel-specific** handler based on `app.focus` (Tree/Detail/Messages)

### Form Editing
Forms use `input_fields: Vec<(String, String)>` with index-based field navigation. Body field (index 0, label "Body") supports multiline: Enter inserts `\n`, Up/Down navigate by line, Home/End move to line start/end. Other fields are single-line. Submit is **F2**, **Ctrl+Enter**, or **Alt+Enter**.

### Keybindings (actual, not README)
**Tree panel:** `j/k` nav, `h/l` collapse/expand, `g/G` jump, `r/F5` refresh, `s` send, `p` peek (prompts count), `d` peek DLQ, `n` create entity, `x` delete entity, `P` clear options  
**Messages panel:** `j/k` nav, `Enter` view detail, `Esc` close detail, `1/2` switch tabs, `e` inline edit & resend, `R` bulk resend DLQ, `D` bulk delete  
**Note:** `x` deletes entities (not `d`); `d` peeks DLQ from tree

### Connection Flow
1. `c` key → if saved connections exist: `ConnectionList` modal (select/delete with `d`/create new with `n`); otherwise: `ConnectionModeSelect`
2. SAS path: `ConnectionInput` → parse connection string → `connect()`
3. Azure AD path: `AzureAdNamespaceInput` → auto-appends `.servicebus.windows.net` if short name → `connect_azure_ad()`
4. Config saved to OS-specific TOML file (see `config.rs:dirs_fallback()`)

### Terminal Safety
`ui/sanitize.rs` strips CSI/OSC escape sequences and control characters from message bodies before rendering. Always use `sanitize_for_terminal()` when displaying untrusted Service Bus message content.

## Adding New Operations

1. Add `BgEvent` variant in `app.rs` if the operation is async
2. Add client method in `client/management.rs` or `client/data_plane.rs`
3. Add key handler in `event.rs` that sets a unique status sentinel via `app.set_status("MyOp...")`
4. In `main.rs:run_app()`, match the sentinel string, spawn the task, send result via `bg_tx`
5. Handle the `BgEvent` in the `bg_rx.try_recv()` match block

For entity form operations, also add `init_*_form()` and `build_*_from_form()` methods on `App`.

## Common Pitfalls

1. **Path casing**: Management API uses `/Subscriptions/`, data plane uses `/subscriptions/`. `normalize_path()` in `data_plane.rs` handles this — don't normalize management paths.
2. **Subscription send routing**: Always use `send_path()` to route sends to the parent topic. Sending directly to a subscription path fails.
3. **Peek increments DeliveryCount**: Each peek-lock cycle increments the broker's delivery count even though messages are abandoned.
4. **Tree/flat node sync**: `app.tree` is hierarchical, `app.flat_nodes` is the linearized view. After `toggle_expand()`, call `rebuild_flat_nodes()`. `build_tree()` returns both.
5. **Sentinel collision**: Multiple `"Submitting..."` dispatches are disambiguated by `app.modal` variant or `app.detail_editing` flag. A new submittable form must add its own `if` guard in `main.rs`.
6. **Topic fan-out**: Any operation that can target a Topic must handle the subscription enumeration pattern. Don't assume single-path operations.
7. **XML namespace prefixes**: `CountDetails` child elements may use `d2p1:` prefix or no prefix depending on Azure's response. Parse helpers try both via `parse_count_details()`.

## Build & Run

```bash
cargo build --release    # Optimized + stripped (lto, strip, codegen-units=1)
cargo run                # Debug build + run
```

Tests exist only in `client/auth.rs` — connection string parsing and SAS token format validation.
