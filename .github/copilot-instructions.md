# Service Bus Explorer TUI — Copilot Instructions

## Project Overview

A cross-platform Rust TUI for managing Azure Service Bus (queues, topics, subscriptions, messages). Built with `ratatui` and direct REST API integration — **no official Azure SDK**.

## Critical Architecture Decisions

### No Azure SDK Dependency
- The Service Bus Rust SDK is unmaintained
- All client code (`src/client/`) uses `reqwest` to call REST APIs directly
- Management plane: ATOM XML feeds (`quick-xml` with targeted parsing)
- Data plane: JSON/HTTP for send/peek/receive operations
- Custom HMAC-SHA256 SAS token generation in `client/auth.rs`

### Async + Sync Hybrid Event Loop
- Main event loop is synchronous (`crossterm` polling in `main.rs`)
- Azure operations spawn `tokio` tasks and communicate via `mpsc` channels
- Pattern: dispatch action → spawn background task → receive `BgEvent` → update state
- See `BgEvent` enum in `app.rs` and task spawning in `main.rs:run_app()`

### ATOM XML Parsing Strategy
- Azure returns inconsistent ATOM feed schemas
- Use **targeted field extraction** with `quick-xml` (not full serde deserialization)
- See `AtomFeed`, `AtomEntry` structs in `client/management.rs`
- Extract only the fields we use; ignore unknown elements to avoid deserialization failures

## Module Structure

```
src/
├── main.rs              # Entry point, async event loop, action dispatch
├── app.rs               # State machine: App, BgEvent, ActiveModal, FocusPanel
├── event.rs             # Keyboard input routing (vim keybindings)
├── config.rs            # TOML persistence (connections, settings)
├── client/              # Azure Service Bus integration
│   ├── auth.rs          # SAS/Azure AD auth, connection string parsing
│   ├── management.rs    # Management plane: entity CRUD (ATOM XML)
│   ├── data_plane.rs    # Data plane: send, peek, receive, purge
│   ├── models.rs        # Entity/message data models
│   └── error.rs         # ServiceBusError with thiserror
└── ui/                  # ratatui rendering components
    ├── layout.rs        # Top-level frame composition
    ├── tree.rs          # Entity tree navigation widget
    ├── messages.rs      # Message list + detail view
    ├── modals.rs        # Dialogs (connection, forms, confirmation)
    ├── detail.rs        # Entity properties/runtime info panel
    └── status_bar.rs    # Bottom status + keybinds
```

## Key Patterns & Conventions

### Background Task Pattern
All Azure operations follow this flow:
1. User triggers action (e.g., peek messages)
2. Status message signals intent (e.g., `app.set_status("Peeking messages...")`)
3. `main.rs` detects status sentinel, spawns `tokio::spawn(async move { ... })`
4. Task sends result via `app.bg_tx.send(BgEvent::...)`
5. Main loop polls `app.bg_rx.try_recv()`, updates state, triggers UI refresh

**Example:** See "Peek messages (spawned)" comment block in `main.rs:run_app()`

### Entity Path Handling
- Queues: `"queue-name"`
- Topics: `"topic-name"`
- Subscriptions: `"topic-name/subscriptions/sub-name"` (note: case-sensitive path segment)
- DLQ paths: append `/$deadletterqueue` to any entity path
- **Send operations:** Subscriptions must be routed to parent topic — use `send_path()` helper to strip `/subscriptions/...` suffix

### State Management
- `App` struct in `app.rs` is the single source of truth
- State changes only via event handlers in `event.rs` or async task results
- Modal overlays: `ActiveModal` enum controls which dialog is shown
- Focus management: `FocusPanel` enum for Tree/Detail/Messages panels

### Error Handling
- Use `ServiceBusError` from `client/error.rs` for client errors
- Display errors to user: `app.set_error(msg)` (sets `status_message`, `is_error = true`)
- Non-blocking: errors don't crash the app, just update status bar

## Development Workflows

### Build & Run
```bash
cargo build               # Debug build
cargo build --release     # Optimized + stripped
cargo run                 # Run directly
./target/release/service-bus-explorer-tui  # Run compiled binary
```

### Testing Azure Integration
- Requires valid Service Bus namespace + connection string
- Connection string format: `Endpoint=sb://<namespace>.servicebus.windows.net/;SharedAccessKeyName=<name>;SharedAccessKey=<key>`
- Azure AD auth: supported via `azure_identity` crate (default credential chain)
- Config stored at OS-specific paths (see `config.rs:dirs_fallback()`)

### Adding New Operations
1. Define new `BgEvent` variant in `app.rs` if async
2. Add client method in `client/management.rs` or `client/data_plane.rs`
3. Wire keyboard handler in `event.rs` to set status sentinel
4. Add task spawn logic in `main.rs:run_app()` matching status
5. Handle `BgEvent` result in main loop's `bg_rx.try_recv()` match

### UI Components
- All rendering in `ui/` modules uses `ratatui::widgets`
- **Never** directly mutate `Frame` outside render functions
- Use `Span`, `Line`, `Paragraph`, `Block` for text formatting
- State-driven rendering: widgets query `&App` state, don't store their own

## Common Pitfalls

1. **Subscription send paths:** Always use `send_path()` helper to route messages to parent topic
2. **XML field casing:** Azure ATOM feeds use `PascalCase` for element names (e.g., `<QueueDescription>`, `<MaxSizeInMegabytes>`)
3. **Modal input routing:** When `app.modal != ActiveModal::None`, input goes to `handle_modal_input()`, not panel handlers
4. **Background cancellation:** Long operations (purge, bulk resend) must check `cancel.load()` periodically
5. **Tree/flat node sync:** `app.tree` is hierarchical, `app.flat_nodes` is linear for selection — keep indices aligned

## File References

- Background task pattern: [src/main.rs](src/main.rs) (search "spawn async")
- Connection string parsing: [src/client/auth.rs](src/client/auth.rs)
- ATOM XML parsing: [src/client/management.rs](src/client/management.rs) (`AtomFeed` structs)
- Vim keybindings: [src/event.rs](src/event.rs) (`handle_tree_input`, `handle_message_input`)
- Message purge concurrency: [src/client/data_plane.rs](src/client/data_plane.rs) (`purge_concurrent` method)
