---
description: 'Add a new modal dialog overlay to the TUI. Use when: add modal, add dialog, add popup, add form modal, add confirmation dialog, new overlay, new input dialog, add prompt dialog, user input form.'
---

# Add Modal Dialog

Adds a new modal overlay (dialog, confirmation, form input) to the TUI. Modals intercept all keyboard input when open and render on top of the main layout.

## Architecture

Modals are driven by the `ActiveModal` enum in `app.rs`. When `app.modal != ActiveModal::None`:
1. All keyboard input routes to `handle_modal_input()` in `event.rs`
2. The modal renders as an overlay in `ui/modals.rs`
3. Closing the modal sets `app.modal = ActiveModal::None`

## Files Touched (in order)

| Step | File | What |
|------|------|------|
| 1 | `src/app.rs` | Add `ActiveModal` variant |
| 2 | `src/app.rs` | (Optional) Add `init_*_form()` and `build_*_from_form()` methods |
| 3 | `src/event.rs` | Add input handling in `handle_modal_input()` |
| 4 | `src/ui/modals.rs` | Add rendering logic |
| 5 | `src/commands.rs` | (If form) Add a `submit_form()` arm for submission |

## Procedure

### Step 1: Add ActiveModal variant

In `src/app.rs`, add the variant:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActiveModal {
    // ... existing variants ...
    MyDialog,
    // Or with associated data:
    MyConfirm { entity_path: String, count: u32 },
}
```

**Variant patterns used in this project:**
- **Simple**: `Help`, `ConnectionModeSelect` — no data, just open/close
- **With context**: `ConfirmDelete(String)` — carries entity path
- **With multi-field data**: `ConfirmBulkResend { entity_path, count, is_topic }` — carries operation context
- **With state machine**: `NamespaceDiscovery { state: DiscoveryState }` — for multi-step flows

### Step 2: Form initialization (if applicable)

If the modal is an input form, add methods on `App`:

```rust
impl App {
    pub fn init_my_form(&mut self) {
        self.input_fields = vec![
            ("Field Name".to_string(), "default_value".to_string()),
            ("Another Field".to_string(), String::new()),
        ];
        self.input_field_index = 0;
        self.form_cursor = 0;
        self.modal = ActiveModal::MyForm;
    }

    pub fn build_my_thing_from_form(&self) -> MyThing {
        let get = |idx: usize| -> Option<String> {
            self.input_fields.get(idx).and_then(|(_, v)| {
                if v.is_empty() { None } else { Some(v.clone()) }
            })
        };
        MyThing {
            field: get(0).unwrap_or_default(),
            // ...
        }
    }
}
```

**Form conventions:**
- Field index 0 with label `"Body"` enables multiline editing (Enter inserts `\n`, Up/Down navigate lines)
- All other fields are single-line
- Submit is **F2**, **Ctrl+Enter**, or **Alt+Enter**
- `input_field_index` tracks the currently focused field
- `form_cursor` tracks cursor position within the field value

### Step 3: Input handling

In `src/event.rs`, add a match arm in `handle_modal_input()`:

```rust
fn handle_modal_input(app: &mut App, key: KeyEvent) {
    match &app.modal {
        // ... existing arms ...
        ActiveModal::MyDialog => match key.code {
            KeyCode::Esc => {
                app.modal = ActiveModal::None;
            }
            KeyCode::Enter => {
                // Handle confirmation / submission
                app.request(Command::MyOperation, "Working...");
                // Or close directly:
                app.modal = ActiveModal::None;
            }
            _ => {}
        },
        // For forms, use the shared form_input_handler pattern:
        ActiveModal::MyForm => {
            handle_form_keys(app, key, |app| {
                // This closure runs on submit (F2/Ctrl+Enter/Alt+Enter)
                app.request(Command::SubmitForm, "Submitting...");
            });
        }
        _ => {}
    }
}
```

**Input routing rules:**
- Esc always closes the modal (set `app.modal = ActiveModal::None`)
- For confirmation dialogs: Enter confirms, any other key is ignored
- For forms: delegate to the shared form key handler for field navigation, then handle submit
- For list-selection modals: j/k or Up/Down navigate, Enter selects

### Step 4: Rendering

In `src/ui/modals.rs`, add a rendering function and call it from the main modal render dispatch:

```rust
pub fn render_my_dialog(frame: &mut Frame, app: &App) {
    let area = centered_rect(60, 40, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" My Dialog ")
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Render content in `inner`
}
```

Then add the dispatch in `render_modal()`:

```rust
pub fn render_modal(frame: &mut Frame, app: &mut App) {
    match &app.modal {
        // ... existing ...
        ActiveModal::MyDialog => render_my_dialog(frame, app),
        _ => {}
    }
}
```

**Rendering conventions:**
- Use `centered_rect(width_pct, height_pct, area)` for modal positioning
- Always render `Clear` widget first to erase background
- Use `Color::Cyan` border for informational, `Color::Yellow` for warnings, `Color::Red` for destructive
- For forms, use the shared `render_form()` helper if available, otherwise render fields manually

### Step 5: Command dispatch (if form submits async work)

Form modals submit with F2 / Ctrl+Enter, which calls `app.request(Command::SubmitForm, ...)`. Add an arm for your modal to `submit_form()` in `src/commands.rs`; it chooses the handler from `app.modal`:

```rust
match app.modal {
    // ... existing forms ...
    ActiveModal::MyForm => submit_my_form(app),
    _ => {}
}
```

Confirmation modals that start an operation should call `app.request(Command::MyOperation, "...")` on `y`, and the dispatcher reads any details it needs from `app.modal`. See the `/add-operation` skill.

## Modal Types Reference

| Type | Example | User Interaction |
|------|---------|------------------|
| Confirmation | `ConfirmDelete` | Enter to confirm, Esc to cancel |
| Selection list | `ConnectionList` | j/k navigate, Enter select, Esc cancel |
| Text input | `ConnectionInput` | Type text, Enter submit, Esc cancel |
| Multi-field form | `SendMessage`, `CreateQueue` | Tab/j/k between fields, F2 submit |
| Info/error display | `Help` | Esc to close |
| Multi-step wizard | `NamespaceDiscovery` | State machine driven |

## Checklist

- [ ] `ActiveModal` variant added to `app.rs`
- [ ] Variant derives `Debug, Clone, PartialEq, Eq` (or associated data does)
- [ ] Input handler added in `handle_modal_input()` in `event.rs`
- [ ] Esc always closes the modal
- [ ] Rendering function added to `ui/modals.rs`
- [ ] Modal dispatched in `render_modal()`
- [ ] Form init/build methods added (if form modal)
- [ ] `submit_form()` arm added in `commands.rs` (if async submission)
