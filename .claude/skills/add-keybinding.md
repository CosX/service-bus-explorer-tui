---
description: 'Add or modify a keyboard shortcut in the TUI. Use when: add keybinding, add shortcut, add key handler, new keyboard command, add hotkey, modify key mapping, input handling, key event.'
---

# Add Keybinding

Adds a new keyboard shortcut to the TUI. The input routing system has a strict priority chain that must be respected.

## Input Routing Priority

`src/event.rs` routes keys in this exact order:

```
1. Background running + Esc    → cancel operation
2. Modal open (modal != None)  → handle_modal_input()
3. Inline editing (detail_editing) → handle_detail_edit_input()
4. Global keys (q, Ctrl+C, ?, c, Tab, BackTab)
5. Panel-specific handler based on app.focus:
   ├── FocusPanel::Tree    → handle_tree_input()
   ├── FocusPanel::Detail  → handle_detail_input()
   └── FocusPanel::Messages → handle_message_input()
```

A key pressed during an active modal will NEVER reach tree/message handlers. A key pressed during inline editing will NEVER reach global handlers.

## Files Touched

| Step | File | What |
|------|------|------|
| 1 | `src/event.rs` | Add key match arm in the appropriate handler |
| 2 | `src/ui/help.rs` | Document the keybinding in the help overlay |
| 3 | (varies) | If the key triggers an async operation, follow the `/add-operation` skill |

## Procedure

### Step 1: Choose the correct handler

| Handler | When to use | Location in `event.rs` |
|---------|-------------|----------------------|
| Global keys block | Keys that work regardless of focused panel | Top of `handle_events()` after modal/edit guards |
| `handle_tree_input()` | Keys for tree panel navigation/actions | Tree-panel match block |
| `handle_detail_input()` | Keys for detail panel | Detail-panel match block |
| `handle_message_input()` | Keys for message list/detail view | Message-panel match block |
| `handle_modal_input()` | Keys within a modal overlay | See `/add-modal` skill |
| `handle_detail_edit_input()` | Keys during inline WYSIWYG editing | Called from `handle_message_input()` when `detail_editing` |

### Step 2: Add the key handler

```rust
fn handle_tree_input(app: &mut App, key: KeyEvent) {
    match key.code {
        // ... existing keys ...
        KeyCode::Char('X') => {
            // Guard for background operations
            if app.bg_running {
                app.set_status("A background operation is in progress...");
            } else if let Some((path, entity_type)) = app.selected_entity() {
                // Validate entity type
                match entity_type {
                    EntityType::Queue | EntityType::Subscription | EntityType::Topic => {
                        // Do the thing
                    }
                    _ => {
                        app.set_status("Select a valid entity");
                    }
                }
            }
        }
        _ => {}
    }
}
```

### Step 3: Update the help overlay

In `src/ui/help.rs`, add the keybinding to the appropriate section.

## Existing Keybindings (avoid collision)

### Global
| Key | Action |
|-----|--------|
| `q` | Quit |
| `Ctrl+C` | Quit |
| `?` | Help overlay |
| `c` | Connection dialog / switch |
| `Tab` | Next panel |
| `Shift+Tab` | Previous panel |

### Tree Panel
| Key | Action |
|-----|--------|
| `j` / `Down` | Navigate down |
| `k` / `Up` | Navigate up |
| `h` / `Left` | Collapse node |
| `l` / `Right` | Expand node |
| `g` | Jump to top |
| `G` | Jump to bottom |
| `r` / `F5` | Refresh tree |
| `s` | Send message |
| `p` | Peek messages (prompts count) |
| `d` | Peek DLQ messages |
| `n` | Create entity |
| `x` | Delete entity |
| `P` | Clear options (purge) |

### Message Panel
| Key | Action |
|-----|--------|
| `j` / `Down` | Navigate list / scroll detail |
| `k` / `Up` | Navigate list / scroll detail |
| `Enter` | View message detail |
| `Esc` | Close detail view |
| `1` | Switch to Messages tab |
| `2` | Switch to Dead Letter tab |
| `e` | Edit & resend message |
| `R` | Bulk resend DLQ |
| `D` | Bulk delete messages |

### Detail Panel
| Key | Action |
|-----|--------|
| `1` | Switch to Messages tab + focus |
| `2` | Switch to DLQ tab + focus |

## Key Selection Guidelines

- **Lowercase** for frequent, non-destructive actions (peek, navigate, send)
- **Uppercase** for destructive or bulk actions (Delete, Resend, Purge)
- **Ctrl+** for system-level actions (quit, force operations)
- **F-keys** for form submission (F2 = submit, F5 = refresh)
- Avoid overloading existing keys — check the collision table above
- Keys behave differently per panel; the same key can have different meanings in tree vs messages

## Checklist

- [ ] Key handler added in the correct function in `event.rs`
- [ ] `bg_running` guard included (if triggering async work)
- [ ] Entity type validated (if operating on entities)
- [ ] No collision with existing keybindings in the same scope
- [ ] Help overlay updated in `ui/help.rs`
- [ ] If async: `Command` + dispatch added per `/add-operation`
