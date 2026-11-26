# Service Bus Explorer TUI — Claude Instructions

## Project Overview

A cross-platform Rust TUI for managing Azure Service Bus (queues, topics, subscriptions, messages). Built with `ratatui` and direct REST API integration — **no official Azure SDK**.

## Architecture

### No Azure SDK
All client code in `src/client/` uses `reqwest` against the REST API directly. Management plane uses ATOM XML feeds parsed with targeted string extraction (not full serde XML). Data plane uses JSON/HTTP. Auth is HMAC-SHA256 SAS tokens or Azure AD Bearer tokens via `azure_identity`.

### Async + Sync Hybrid Event Loop
The main loop in `main.rs` polls `crossterm` events synchronously at 100ms intervals. Azure operations are dispatched as `tokio::spawn` tasks that communicate results back via an unbounded `mpsc` channel (`app.bg_tx` / `app.bg_rx`). The main loop drains `bg_rx.try_recv()` each tick and hands each event to `bg_events::handle()`. Purge progress uses a shared `AtomicU64` sampled by a reporter task, so nothing queues up however fast messages are deleted.

### Command Dispatch
Input handlers request work with `app.request(Command::X, "status text")`, which stores `app.pending_command`. Each tick `main.rs` takes it and calls `commands::dispatch()`, which spawns the task. The status text is display-only; **never match on `app.status_message` to trigger work**.
- `Command::SubmitForm` is disambiguated by `app.modal` (or `app.detail_editing`) inside `commands::submit_form()`
- Confirm modals (`ConfirmBulkResend`, `ConfirmBulkDelete`, `ConfirmSingleDelete`) stay open until dispatch; the dispatcher reads their fields from `app.modal`
- Commands that must not overlap (purge, resend, bulk delete, ...) are listed in `dispatch()`'s `exclusive` check and rejected while `bg_running`

When adding new operations, add a `Command` variant, request it from `event.rs`, and add a `dispatch` arm in `commands.rs`.

### ATOM XML Parsing
Azure returns inconsistent ATOM feed schemas. Parsing in `management.rs` uses raw string search (`extract_element`, `extract_element_value`, `extract_entries`) rather than serde XML deserialization. `CountDetails` child elements may be prefixed (`d2p1:ActiveMessageCount`) or unprefixed — the parser tries both.
- **Feeds are paged**: Azure returns at most 100 entries per request. List calls go through `get_atom_feed()`, which follows `$skip`/`$top`.
- **Escaping**: element text is decoded with `unesc()` on read; values written into entity XML go through `esc()`. SQL filters are written inside CDATA.
- **Rule filters** carry a `RuleFilterKind`; only SQL/True/False filters can be edited as SQL (correlation filters are refused).

## Module Structure

```
src/
├── main.rs              # Entry point, terminal setup + panic hook, event loop
├── mouse.rs             # Hit map + mouse handling (clicks replayed as keys)
├── commands.rs          # Command dispatch → spawned Azure operations
├── bg_events.rs         # Applies BgEvent results to App state
├── loaders.rs           # Tree refresh, detail pane, metrics (with backoff), ARM resource ID
├── bulk_ops.rs          # Fan-out path resolution, DLQ resend loop, selective resend/delete
├── app.rs               # App state, BgEvent + Command enums, form builders, tree construction
├── event.rs             # Input routing: global → modal → panel handlers
├── config.rs            # TOML persistence (connections, settings, OS-specific paths)
├── client/
│   ├── auth.rs          # SAS token gen, Azure AD token, connection string parsing
│   ├── management.rs    # Management plane: ATOM XML CRUD + raw XML parsing helpers
│   ├── data_plane.rs    # Data plane: send, peek-lock, receive-delete, purge
│   ├── scan.rs          # Lock-based peek / find-by-sequence algorithms + fake broker for tests
│   ├── resource_manager.rs # ARM: namespace discovery, resource ID, Azure Monitor metrics
│   ├── models.rs        # Entity descriptions, message models, TreeNode/FlatNode
│   └── error.rs         # ServiceBusError and ArmError (thiserror)
└── ui/
    ├── layout.rs        # Top-level 3-panel layout (tree | detail | messages)
    ├── tree.rs          # Entity tree with inline message/DLQ counts
    ├── messages.rs      # Message list + detail view + inline edit rendering
    ├── modals.rs        # Connection, form, confirm, clear-options, peek-count dialogs
    ├── buttons.rs       # Clickable button rows for dialogs and forms
    ├── detail.rs        # Entity properties/runtime info panel + sparkline previews
    ├── metrics_detail.rs # Full-screen metrics overlay with braille line charts
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

This pattern lives in `bulk_ops::resolve_purge_paths()` / `resolve_resend_pairs()` and the topic-DLQ peek in `commands.rs`. It calls `mgmt.list_subscriptions()` first, then iterates.

### Peek Implementation
The REST API's `PeekOnly=true` has no cursor (always returns the same first message). Peek is implemented as **peek-lock + abandon** (`scan::peek`): lock N messages sequentially, collect them, then abandon all locks. This makes messages available again but **increments `DeliveryCount`** on each peek.
- On active queues/subscriptions, releasing a message whose `DeliveryCount` has reached `MaxDeliveryCount` **dead-letters it**. The peek is given the entity's limit (`bulk_ops::delivery_guard`), stops at the first such message and reports it. DLQs have no limit.
- Locks are released on every exit path, including errors.

### Message Bodies and Properties
- `ReceivedMessage.raw_body` holds the exact bytes; `body` is a lossy UTF-8 view for display. Resends use `to_sendable()`, which copies the raw bytes. Binary bodies cannot be opened for editing.
- User properties are kept as wire literals (`raw_custom_properties`: `"text"`, `42`, `true`) so a resend keeps their types; `custom_properties` holds display values. `to_sendable()` drops `DeadLetterReason` / `DeadLetterErrorDescription`.

### Targeted Removal / Resend
Removing or resending specific messages (`x`, `D`, `R`, edit-resend) scans the entity with `scan::scan_for_sequences`: lock forward, act on targets, hold the rest, release at the end. It is bounded (`DEFAULT_MAX_SCAN`), stops if it wraps around (an expired lock), and reports targets not found. `D` deletes **only the peeked messages**; whole-entity purges are `P` (clear options).

### DLQ Resend Loop
`bulk_ops::resend_dlq_loop` always terminates: failed sends keep their lock so the loop moves on, a path is abandoned after `MAX_CONSECUTIVE_SEND_FAILURES`, and an already-seen sequence number ends the path instead of resending it twice.

### Mouse
Renderers register clickable areas in `app.hits` (`mouse::HitMap`) every frame: panels, tree rows (plus the ▶/▼ expander), message rows and tabs, dialog list rows, form fields, and buttons (`ui/buttons.rs`). `mouse::handle` finds the topmost target under the pointer; while a dialog is open only targets registered after `hits.begin_dialog()` count. Clicks that mean a key (buttons, double-click to open, tabs) are replayed through `event::handle_key`, so mouse and keyboard share one code path. When adding a list, form or button, register it with `app.hits` after rendering it (use the widget state's `offset()` for scrolled lists).

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
**Tree panel:** `j/k` nav, `h/l` collapse/expand, `g/G` jump, `r/F5` refresh, `s` send, `p` peek (prompts count), `d` peek DLQ, `n` create entity, `x` delete entity, `P` clear options (on a Queues/Topics folder node it batches across every queue/topic, with a second confirm)
**Messages panel:** `j/k` nav, `Enter` view detail, `Esc` close detail, `1/2` switch tabs, `e` inline edit & resend, `R` bulk resend DLQ, `D` bulk delete
**Metrics:** `m` toggle metrics on/off, `M` cycle time window (1h/6h/24h/7d), `V` open metrics detail overlay (braille line charts for all 5 metrics)
**Note:** `x` deletes entities (not `d`); `d` peeks DLQ from tree

### Connection Flow
1. `c` key → if saved connections exist: `ConnectionList` modal (select/delete with `d`/create new with `n`); otherwise: `ConnectionModeSelect`
2. SAS path: `ConnectionInput` → parse connection string → `connect()`
3. Azure AD path: `AzureAdNamespaceInput` → auto-appends `.servicebus.windows.net` if short name → `connect_azure_ad()`
4. Config saved to OS-specific TOML file (see `config.rs:dirs_fallback()`)

### Terminal Safety
`ui/sanitize.rs` strips CSI/OSC escape sequences, control characters and bidi overrides before rendering. Always use `sanitize_for_terminal()` (or `value_with_cursor()` for form fields) when displaying untrusted Service Bus content — including edit-form values copied from a message.

### Azure Monitor Metrics
Requires Azure AD authentication (not available with SAS keys). The ARM resource ID is resolved once per connection via `ResourceManagerClient::resolve_namespace_resource_id()`.

**Failure backoff:** a failed metrics fetch is remembered in `app.metrics_failure` and not retried for that entity for 30s (5 min for permission errors). `m` / `M` clear it.

**Two-tier UI:**
- **Detail pane preview**: small sparkline charts for Active Messages and Dead-letter, rendered inline below entity properties (`detail.rs`)
- **Metrics detail overlay** (`V` key): full-screen modal with braille dot line charts (`Chart` + `Marker::Braille` + `GraphType::Line`) for all 5 metrics, with y-axis labels and cur/peak values (`metrics_detail.rs`)

**Five metrics fetched from Azure Monitor** (via `resource_manager.rs:query_entity_metrics()`):
- `ActiveMessages`, `DeadletteredMessages`, `ScheduledMessages` — gauge metrics (average aggregation)
- `IncomingMessages`, `OutgoingMessages` — throughput metrics (total aggregation)

Gauge and throughput metrics require different aggregation types, so they are fetched in two parallel requests using `tokio::join!`.

**Time windows** (`MetricsWindow` enum): 1h (PT1M interval), 6h (PT5M), 24h (PT1H), 7d (PT1H). Cycled with `M` key. The overlay also supports `M` to cycle while open.

**App state**: `metrics: MetricsAvailability` (`Unsupported` / `RetryRequested` / `Resolving` / `Available` / `Failed(reason)`; `m` retries a failed resolution), `metrics_enabled` (user toggle), `metrics_window`, `metrics_pending` (fetch in progress), `metrics_failure` (backoff), `entity_metrics` (cached data). Metrics are re-fetched when the selected entity changes, the window cycles, or metrics are toggled back on.

## Adding New Operations

1. Add a `Command` variant (and a `BgEvent` variant for the result) in `app.rs`
2. Add client method in `client/management.rs` or `client/data_plane.rs` (lock-taking logic goes in `client/scan.rs` against its traits)
3. Add key handler in `event.rs` that calls `app.request(Command::MyOp, "My operation...")`
4. Add a `dispatch` arm in `commands.rs` that spawns the task and sends the result via `bg_tx`
5. Handle the `BgEvent` in `bg_events.rs`

For entity form operations, also add `init_*_form()` and `build_*_from_form()` methods on `App`, and a `submit_form()` arm in `commands.rs`.

Use the `/add-operation` skill for the full step-by-step procedure.

## Engineering Standards

Optimize for: correctness, maintainability, security, performance, operational reliability, and long-term cost of ownership. Never optimize for speed of writing code if it increases system entropy.

When working on any task:
1. Identify the actual engineering problem, not just the surface request
2. Detect missing constraints (scale, concurrency, lifecycle, security)
3. Reject fragile or short-sighted approaches
4. Propose production-grade solutions first; simplified alternatives only if explicitly requested

### Rust Standards
- Use enums and the type system for state modeling — not booleans or string flags
- Prefer owned types at API boundaries, borrows internally
- `Result` and `Option` everywhere — no `unwrap()`/`expect()` outside test code
- Explicit error types (`thiserror`) over stringly-typed errors
- Avoid unnecessary clones — justify each one
- No blocking calls in async context
- Drop-based resource cleanup; `CancellationToken` for graceful shutdown
- Bounded channels and semaphores for concurrency control

### Architecture
Enforce: clear boundaries, separation of concerns, testability, failure isolation.

Prevent: god modules, leaky abstractions between client/UI/app layers, unbounded resource usage, UI rendering coupled to business logic, panic-driven error handling, nested tokio runtimes.

### Testing
Require unit tests for domain logic, integration tests for infrastructure behavior. Tests must validate behavior not implementation details. Reject tests that mock everything, depend on timing, or assert internal calls instead of outcomes.

## Available Skills

- `.claude/skills/add-operation.md` — Add a new async background operation using the `Command` dispatch pattern
- `.claude/skills/add-keybinding.md` — Add or modify a keyboard shortcut
- `.claude/skills/add-modal.md` — Add a new modal dialog overlay
- `.claude/skills/add-ui-panel.md` — Add or modify a UI panel or rendering component
- `.claude/skills/sb-rest-client.md` — Extend the Azure Service Bus REST API client
- `.claude/skills/code-review.md` — Structured production-grade code review
- `.claude/skills/rust-check.md` — Check code against Rust standards for this project
- `.claude/skills/security-check.md` — Security-focused evaluation of code

## Common Pitfalls

1. **Path casing**: Management API uses `/Subscriptions/`, data plane uses `/subscriptions/`. `normalize_path()` in `data_plane.rs` handles this — don't normalize management paths.
2. **Subscription send routing**: Always use `send_path()` to route sends to the parent topic. Sending directly to a subscription path fails.
3. **Peek increments DeliveryCount**: Each peek-lock cycle increments the broker's delivery count even though messages are abandoned.
4. **Tree/flat node sync**: `app.tree` is hierarchical, `app.flat_nodes` is the linearized view. After `toggle_expand()`, call `rebuild_flat_nodes()`. `build_tree()` returns both.
5. **Form submission**: `Command::SubmitForm` is disambiguated by `app.modal` variant or `app.detail_editing` flag. A new submittable form must add its own arm in `commands::submit_form()`.
6. **Topic fan-out**: Any operation that can target a Topic must handle the subscription enumeration pattern. Don't assume single-path operations.
7. **XML namespace prefixes**: `CountDetails` child elements may use `d2p1:` prefix or no prefix depending on Azure's response. Parse helpers try both via `parse_count_details()`.
8. **Subscription resends fan out**: sending to a topic reaches every matching subscription. Confirmations show `App::resend_fan_out_warning()`.
9. **Cursor positions are byte offsets**: always clamp with `ui::sanitize::floor_char_boundary` before slicing form text; move between lines by char column.
10. **Config file**: never replace a config that failed to parse. `AppConfig::load` moves it aside; saves are atomic and owner-only (0600).

## Build & Run

```bash
cargo build --release    # Optimized + stripped (lto, strip, codegen-units=1)
cargo run                # Debug build + run
```

Unit tests cover connection parsing/SAS tokens, XML parsing fixtures (`management.rs`), the lock-based algorithms and resend loop against an in-memory fake broker (`client/scan.rs`, `bulk_ops.rs`), message models, config load/save, cursor handling and sanitization. CI runs `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings -A dead_code -A unused`, `cargo build --release` and `cargo test`.
