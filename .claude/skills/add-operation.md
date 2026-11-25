---
description: 'Add a new async background operation to the TUI using the Command dispatch pattern. Use when: add new operation, new async task, new background action, new Command, new BgEvent, spawn async operation, add feature that calls Azure Service Bus API.'
---

# Add Async Operation

Adds a new background operation using the `Command` dispatch pattern. This is the central workflow for any feature that calls the Azure Service Bus API (or any async work) from the TUI.

## Architecture

The TUI uses a sync/async hybrid event loop:
1. `event.rs` / `event_modal.rs` handle keystrokes and call `app.request(Command::X, "status text")`
2. `main.rs` takes `app.pending_command` each tick and calls `commands::dispatch()`
3. `commands.rs` spawns a `tokio::spawn` task; the task sends its result via `app.bg_tx`
4. `main.rs` drains `app.bg_rx.try_recv()` and passes each event to `bg_events::handle()`

The status bar text is display-only. Never match on `app.status_message` to trigger work.

## Files Touched (in order)

| Step | File | What |
|------|------|------|
| 1 | `src/app.rs` | Add a `Command` variant (and a `BgEvent` variant for the result) |
| 2 | `src/client/management.rs` or `src/client/data_plane.rs` | Add client method |
| 3 | `src/event.rs` / `src/event_modal.rs` | Key handler calls `app.request(Command::MyOp, "My operation...")` |
| 4 | `src/commands.rs` | Add a `dispatch` arm and a function that spawns the task |
| 5 | `src/bg_events.rs` | Handle the `BgEvent` variant |
| 6 | `src/ui/help.rs` | Document the new keybinding in the help overlay |

## Procedure

### Step 1: Define the Command and BgEvent variants

In `src/app.rs`:

```rust
pub enum Command {
    // ... existing variants ...
    MyOperation,
}

pub enum BgEvent {
    // ... existing variants ...
    MyOperationComplete {
        result_field: String,
        warnings: Vec<String>,
    },
}
```

**Rules:**
- Commands carry what the key handler knows (e.g. `Peek { count, dlq }`). If the data lives in an open confirm modal, `commands.rs` can read it from `app.modal`.
- If the operation returns large data, `Box` it (see `DetailLoaded { view: Box<DetailView> }`)
- If the operation can partially succeed, include counts and `warnings: Vec<String>` (see `ResendComplete`)
- Progress updates reuse the existing `Progress(String)` variant

### Step 2: Add the client method

- Management plane (ATOM XML CRUD) → `src/client/management.rs`. List feeds must go through `get_atom_feed()` (paged; Azure caps pages at 100).
- Data plane (messages, peek, send, purge) → `src/client/data_plane.rs`
- Anything that locks messages → write it against the `LockingReceiver` / `MessageSender` traits in `src/client/scan.rs` so it can be unit-tested with the fake broker, and release every lock on every exit path.

**Rules:**
- Return `Result<T>` using the crate's `ServiceBusError` (ARM calls use `ArmError`)
- Management API uses PascalCase paths (`/Subscriptions/`), data plane uses lowercase (`/subscriptions/`)
- XML text written to the management API must go through `esc()`; text read back through `unesc()`

### Step 3: Add the key handler

```rust
KeyCode::Char('X') => {
    if !block_if_bg_running(app, BG_BUSY_MSG) {
        if let Some((_, entity_type)) = app.selected_entity() {
            match entity_type {
                EntityType::Queue | EntityType::Subscription | EntityType::Topic => {
                    app.request(Command::MyOperation, "My operation...");
                }
                _ => app.set_status("Select a valid entity"),
            }
        }
    }
}
```

For operations that need confirmation or input, open a modal; the modal's confirm key calls `app.request(...)`.

### Step 4: Dispatch and spawn in commands.rs

Add the arm to `dispatch()`. If the operation must not overlap other background work, add it to the `exclusive` list there.

```rust
Command::MyOperation => my_operation(app),

fn my_operation(app: &mut App) {
    let Some(dp) = require_data_plane(app) else { return };
    let Some((path, _)) = app.selected_entity() else { return };
    let entity_path = path.to_string();
    let tx = app.bg_tx.clone();
    app.set_status("Running...");

    tokio::spawn(async move {
        let event = match dp.my_method(&entity_path).await {
            Ok(result) => BgEvent::MyOperationComplete { result_field: result, warnings: vec![] },
            Err(e) => BgEvent::Failed(format!("Operation failed: {}", e)),
        };
        let _ = tx.send(event);
    });
}
```

**Rules:**
- No `unwrap()`: use `let ... else` on `app.data_plane` / `app.management`
- For cancellable long-running operations, use `app.new_cancel_token()` and set `app.bg_running = true`
- Always send exactly one final event (`Complete`, `Failed` or `Cancelled`)

### Step 5: Handle the BgEvent result

In `bg_events::handle()`:

```rust
BgEvent::MyOperationComplete { result_field, warnings } => {
    let mut status = format!("Operation complete: {}", result_field);
    append_warnings(&mut status, &warnings);
    app.set_status(status);
    app.bg_running = false;
    needs_refresh = true; // if tree counts may have changed
}
```

### Step 6: Document the keybinding

Add the new keybinding to `src/ui/help.rs` in the appropriate section.

## Topic Fan-Out Pattern

If the operation can target a Topic, it must fan out across all subscriptions. Prefer `bulk_ops::resolve_purge_paths()` / `resolve_resend_pairs()`; they also cover `AllQueues` / `AllTopics`.

For DLQ operations on topics, append `/$deadletterqueue` to each subscription path. For sends, route to the parent topic with `send_path_owned()`, and remember that every matching subscription receives the copy (see `App::resend_fan_out_warning`).

## Checklist

- [ ] `Command` and `BgEvent` variants added to `app.rs`
- [ ] Client method added and returns `Result<T>`
- [ ] Key handler guards `bg_running` and validates entity type
- [ ] `dispatch` arm added (and `exclusive` list if needed)
- [ ] Spawn block clones clients and `bg_tx` before `move`; no `unwrap()`
- [ ] Error path sends `BgEvent::Failed`; partial failures go in `warnings`
- [ ] `BgEvent` handled in `bg_events.rs`, `bg_running` reset
- [ ] Topic fan-out handled (if applicable)
- [ ] Lock-taking code releases every lock and has a fake-broker test
- [ ] Help overlay updated with new keybinding
