# Service Bus Explorer TUI

A cross-platform terminal UI for managing Azure Service Bus namespaces — queues, topics, subscriptions, and messages.

Built with Rust, [ratatui](https://ratatui.rs), and the Azure Service Bus REST API (no SDK dependency).

![Rust](https://img.shields.io/badge/rust-1.70%2B-orange)
![License](https://img.shields.io/badge/license-MIT-blue)

## Features

- Browse queues, topics, and subscriptions in a navigable tree
- View entity properties and runtime metrics (message counts, sizes)
- Peek messages and dead-letter queues
- Send messages with custom properties
- Create and delete queues, topics, and subscriptions
- Purge messages from entities
- Multiple saved connections with config persistence
- Clipboard support for copying message bodies
- Vim-style keybindings

## Prerequisites

- **Rust 1.70+** — install via [rustup](https://rustup.rs):
  ```
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```
- **An Azure Service Bus namespace** with a connection string (Shared Access Signature)

## Build

```bash
# Clone the repo
git clone https://github.com/your-org/service-bus-explorer-tui.git
cd service-bus-explorer-tui

# Debug build
cargo build

# Release build (optimised, stripped)
cargo build --release
```

The release binary is at `target/release/service-bus-explorer-tui`.

## Run

```bash
# Run directly via cargo
cargo run

# Or run the compiled binary
./target/release/service-bus-explorer-tui
```

On launch you'll see an empty tree panel. Press **`c`** to open the connection dialog.

### Connect to a namespace

1. Press **`c`** to open the connection prompt.
2. Paste your Service Bus connection string:
   ```
   Endpoint=sb://<namespace>.servicebus.windows.net/;SharedAccessKeyName=RootManageSharedAccessKey;SharedAccessKey=<key>
   ```
3. Press **Enter**. The entity tree loads automatically.

The connection is saved to the config file so you can reconnect on next launch.

### Config file location

| OS      | Path                                                        |
|---------|-------------------------------------------------------------|
| Linux   | `~/.config/sb-explorer/config.toml`                         |
| macOS   | `~/Library/Application Support/sb-explorer/config.toml`     |
| Windows | `%APPDATA%\sb-explorer\config.toml`                         |

## Keyboard shortcuts

### Navigation

| Key              | Action                  |
|------------------|-------------------------|
| `↑` / `k`       | Move up                 |
| `↓` / `j`       | Move down               |
| `←` / `h`       | Collapse node           |
| `→` / `l`       | Expand node             |
| `Enter`          | Select / expand         |
| `g` / `G`       | Jump to first / last    |
| `Tab`            | Next panel              |
| `Shift+Tab`      | Previous panel          |

### Connection

| Key              | Action                  |
|------------------|-------------------------|
| `c`              | Connect / switch        |
| `r` / `F5`      | Refresh entity tree     |

### Entity operations

| Key              | Action                  |
|------------------|-------------------------|
| `n`              | Create new entity       |
| `d`              | Delete selected entity  |

### Message operations

| Key              | Action                  |
|------------------|-------------------------|
| `p`              | Peek messages           |
| `s`              | Send message            |
| `P` (shift)     | Purge all messages      |
| `1` / `2`       | Switch Messages / DLQ   |
| `Enter`          | View message detail     |
| `Esc`            | Close detail view       |

### General

| Key              | Action                  |
|------------------|-------------------------|
| `?`              | Show help overlay       |
| `q` / `Ctrl+C`  | Quit                    |

## Architecture

```
src/
├── main.rs              # Entry point, async event loop, action dispatch
├── app.rs               # Application state machine
├── event.rs             # Keyboard input handling
├── config.rs            # TOML config persistence
├── client/
│   ├── auth.rs          # SAS token generation, connection string parsing
│   ├── models.rs        # Data models (queues, topics, subscriptions, messages)
│   ├── management.rs    # Management plane (ATOM XML) — entity CRUD
│   ├── data_plane.rs    # Data plane — send, peek, receive, purge
│   └── error.rs         # Error types
└── ui/
    ├── layout.rs        # Top-level layout composition
    ├── tree.rs          # Entity tree widget
    ├── detail.rs        # Property/runtime info panel
    ├── messages.rs      # Message list and detail view
    ├── modals.rs        # Connection, form, and confirm dialogs
    ├── status_bar.rs    # Status bar
    └── help.rs          # Keyboard shortcut overlay
```

### Design decisions

- **No Azure SDK** — the official Rust SDK for Service Bus is unmaintained. The client layer uses `reqwest` against the REST API directly with HMAC-SHA256 SAS token auth.
- **Synchronous event loop with async dispatch** — keyboard events are polled synchronously via `crossterm`; Service Bus API calls are dispatched as `async` operations within the same `tokio` runtime.
- **ATOM XML parsing** — the management plane returns Atom feeds. Parsed with targeted string extraction rather than full serde XML deserialization to handle the inconsistent schema Azure returns.

## License

[MIT](LICENSE)
