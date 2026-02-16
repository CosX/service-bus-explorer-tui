# Service Bus Explorer TUI

A cross-platform terminal UI for managing Azure Service Bus namespaces — queues, topics, subscriptions, and messages. Inspired by the wonderful [ServiceBusExplorer](https://github.com/paolosalvatori/ServiceBusExplorer) application and bringing its functionality cross-platform.

Built with Rust, [ratatui](https://ratatui.rs), and the Azure Service Bus REST API (no SDK dependency).

![Rust](https://img.shields.io/badge/rust-1.70%2B-orange)
![License](https://img.shields.io/badge/license-MIT-blue)

## Features

- Browse queues, topics, and subscriptions in a navigable tree with inline message counts
- View entity properties and runtime metrics (active, DLQ, scheduled, transfer counts)
- Peek messages and dead-letter queues (with configurable count)
- Send messages with custom properties, content type, TTL, session ID, and more
- Edit & resend messages inline (WYSIWYG) — including DLQ messages back to the main entity
- Create and delete queues, topics, and subscriptions
- Purge messages — concurrent delete, DLQ clear, or DLQ resend (with progress & cancellation)
- Bulk resend DLQ → main entity and bulk delete from messages panel
- Topic operations automatically fan out across all subscriptions
- Multiple saved connections with config persistence (SAS and Azure AD)
- Azure AD (Microsoft Entra ID) authentication via default credential chain
- Vim-style keybindings
- Terminal escape injection protection for untrusted message content

## Installation

### Homebrew (macOS/Linux)

The fastest way to install on macOS or Linux using Homebrew:

```bash
# Add the tap (first time only)
brew tap OWNER/tap

# Install
brew install OWNER/tap/service-bus-explorer-tui
```

The `brew tap` command adds a third-party repository to Homebrew. After tapping, you can install and update the tool like any other Homebrew package.

### cargo-binstall

Fast installation of pre-built binaries via `cargo-binstall`:

```bash
cargo binstall service-bus-explorer-tui
```

[cargo-binstall](https://github.com/cargo-bins/cargo-binstall) downloads pre-compiled binaries instead of building from source, saving significant time. Install `cargo-binstall` first if you don't have it:

```bash
cargo install cargo-binstall
```

### cargo install

Install from source via [crates.io](https://crates.io):

```bash
cargo install service-bus-explorer-tui
```

This compiles from source and installs to `~/.cargo/bin/` (ensure it's in your `PATH`). Requires Rust 1.70+.

### Pre-built binaries

Download pre-built binaries directly from the [GitHub Releases](https://github.com/OWNER/service-bus-explorer-tui/releases) page.

Available platforms:

| Platform              | Artifact                                              |
|-----------------------|-------------------------------------------------------|
| **Linux (x86_64)**    | `service-bus-explorer-tui-x86_64-unknown-linux-gnu`   |
| **Linux (ARM64)**     | `service-bus-explorer-tui-aarch64-unknown-linux-gnu`  |
| **macOS (Intel)**     | `service-bus-explorer-tui-x86_64-apple-darwin`        |
| **macOS (Apple Silicon)** | `service-bus-explorer-tui-aarch64-apple-darwin`   |
| **Windows (x86_64)**  | `service-bus-explorer-tui-x86_64-pc-windows-msvc.exe` |

**Extract and install:**

```bash
# Linux/macOS
tar xzf service-bus-explorer-tui-*.tar.gz
chmod +x service-bus-explorer-tui
sudo mv service-bus-explorer-tui /usr/local/bin/

# Windows: move the .exe to a directory in your PATH
```

### Build from source

Clone and build manually:

```bash
# Clone the repository
git clone https://github.com/OWNER/service-bus-explorer-tui.git
cd service-bus-explorer-tui

# Debug build
cargo build

# Release build (optimized, stripped)
cargo build --release
```

The release binary is at `target/release/service-bus-explorer-tui`.

**Requirements:** Rust 1.70+ — install via [rustup](https://rustup.rs):
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

## Prerequisites

- **An Azure Service Bus namespace** with either:
  - A SAS connection string, or
  - Azure AD credentials (via environment, CLI, managed identity, etc.)

## Run

```bash
# If installed via package manager or cargo
service-bus-explorer-tui

# Or run directly from source
cargo run

# Or run the compiled binary
./target/release/service-bus-explorer-tui
```

On launch you'll see an empty tree panel. Press **`c`** to open the connection dialog.

### Connect to a namespace

#### SAS connection string

1. Press **`c`** to open the connection dialog.
2. If you have saved connections, select one or press **`n`** to add a new one.
3. Choose **SAS** and paste your connection string:
   ```
   Endpoint=sb://<namespace>.servicebus.windows.net/;SharedAccessKeyName=RootManageSharedAccessKey;SharedAccessKey=<key>
   ```
4. Press **Enter**. The entity tree loads automatically.

#### Azure AD (Microsoft Entra ID)

1. Press **`c`** → choose **Azure AD**.
2. Enter your namespace name (e.g. `mynamespace` — `.servicebus.windows.net` is appended automatically).
3. Press **Enter**. Authentication uses the default credential chain (`azure_identity`).

Connections are saved to the config file for reconnection on next launch.

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
| `c`              | Connect / manage connections |
| `r` / `F5`      | Refresh entity tree     |

### Tree panel — entity operations

| Key              | Action                             |
|------------------|------------------------------------|
| `n`              | Create new entity                  |
| `x`              | Delete selected entity             |
| `s`              | Send message to queue/topic        |
| `p`              | Peek messages (prompts for count)  |
| `d`              | Peek dead-letter queue             |
| `P` (shift)      | Clear entity (delete / DLQ resend) |

### Messages panel

| Key              | Action                                   |
|------------------|------------------------------------------|
| `1` / `2`       | Switch Messages / DLQ tab                 |
| `Enter`          | View message detail                      |
| `Esc`            | Close detail view                        |
| `e`              | Edit & resend message (inline WYSIWYG)   |
| `R` (shift)      | Bulk resend all DLQ → main entity        |
| `D` (shift)      | Bulk delete all visible messages         |

### Form editing (send / create / edit)

| Key                        | Action                     |
|----------------------------|----------------------------|
| `Tab` / `↑` / `↓`         | Navigate between fields    |
| `Enter` (in Body field)   | Insert newline             |
| `F2` / `Ctrl+Enter`       | Submit form                |
| `Esc`                      | Cancel                     |

### General

| Key              | Action                  |
|------------------|-------------------------|
| `?`              | Show help overlay       |
| `q` / `Ctrl+C`  | Quit                    |
| `Esc`            | Cancel background operation |

## Architecture

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
    ├── help.rs          # Full keyboard shortcut overlay
    └── sanitize.rs      # Terminal escape injection prevention (CSI/OSC stripping)
```

### Design decisions

- **No Azure SDK** — the official Rust SDK for Service Bus is unmaintained. The client layer uses `reqwest` against the REST API directly with HMAC-SHA256 SAS token auth or Azure AD Bearer tokens.
- **Synchronous event loop with async dispatch** — keyboard events are polled synchronously via `crossterm` at 100ms intervals; Service Bus API calls are spawned as `tokio` tasks that report results back through an `mpsc` channel.
- **ATOM XML parsing** — the management plane returns Atom feeds with inconsistent schemas. Parsed with targeted string extraction (`extract_element`, `extract_element_value`) rather than full serde XML deserialization.
- **Peek via peek-lock + abandon** — the REST API's `PeekOnly=true` has no cursor, so peek is implemented as peek-lock N messages then abandon all locks. This increments `DeliveryCount` on each peek.
- **Concurrent purge** — message deletion spawns multiple parallel receive-and-delete workers (default 32) with progress reporting and cancellation support.

## License

[MIT](LICENSE)


## Disclaimer

This application has been developed using coding agents. Use at own risk.
