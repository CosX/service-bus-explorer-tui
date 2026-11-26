//! Mouse support.
//!
//! Renderers record what is clickable in a [`HitMap`] every frame; this module
//! maps mouse events onto those targets. Where a click means the same thing as
//! a key (a dialog button, opening a message, submitting a form), it is
//! replayed as that key, so mouse and keyboard share one code path.

use std::time::{Duration, Instant};

use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
use ratatui::layout::{Position, Rect};

use crate::app::{ActiveModal, App, FocusPanel, MessageTab};

/// Two clicks on the same target within this window count as a double click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// Something on screen that reacts to the mouse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// A main panel; clicking focuses it, the wheel scrolls it.
    Panel(FocusPanel),
    /// A tree row. `on_toggle` is the ▶/▼ expander.
    TreeRow {
        index: usize,
        on_toggle: bool,
    },
    /// A message list row (index into the filtered view).
    MessageRow(usize),
    MessageTab(MessageTab),
    /// A row of a list inside a dialog; clicking selects, double-click activates.
    DialogListRow(usize),
    /// A form field; clicking moves the input focus to it.
    FormField(usize),
    /// A button or option; clicking presses this key.
    Key(KeyCode),
}

#[derive(Debug, Clone, Copy)]
struct Hit {
    area: Rect,
    target: Target,
    /// Registered while a dialog was being drawn.
    in_dialog: bool,
}

/// Clickable areas of the last frame, topmost last.
#[derive(Debug, Default)]
pub struct HitMap {
    hits: Vec<Hit>,
    drawing_dialog: bool,
}

impl HitMap {
    /// Start a new frame.
    pub fn clear(&mut self) {
        self.hits.clear();
        self.drawing_dialog = false;
    }

    /// Everything registered from now on belongs to the open dialog.
    pub fn begin_dialog(&mut self) {
        self.drawing_dialog = true;
    }

    pub fn add(&mut self, area: Rect, target: Target) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        self.hits.push(Hit {
            area,
            target,
            in_dialog: self.drawing_dialog,
        });
    }

    /// Register one row (`row` lines below `area.y`) of `area`, if it fits.
    pub fn add_row(&mut self, area: Rect, row: u16, target: Target) {
        if row < area.height {
            self.add(
                Rect {
                    x: area.x,
                    y: area.y + row,
                    width: area.width,
                    height: 1,
                },
                target,
            );
        }
    }

    /// Topmost target under the point. While a dialog is open, only the
    /// dialog's own targets count, so clicks never reach the panels beneath.
    fn at(&self, column: u16, row: u16, dialog_open: bool) -> Option<Target> {
        let point = Position::new(column, row);
        self.hits
            .iter()
            .rev()
            .filter(|h| h.in_dialog == dialog_open)
            .find(|h| h.area.contains(point))
            .map(|h| h.target)
    }
}

/// Remembers the last click to detect double clicks.
#[derive(Debug, Default)]
pub struct ClickTracker {
    last: Option<(Target, Instant)>,
}

impl ClickTracker {
    /// Record a click on `target`; true if it completes a double click.
    fn click(&mut self, target: Target) -> bool {
        let now = Instant::now();
        let double = matches!(self.last, Some((t, at)) if t == target && now - at <= DOUBLE_CLICK);
        // A double click is consumed, so a third click starts over.
        self.last = if double { None } else { Some((target, now)) };
        double
    }
}

/// Replay a key press. Quitting is never mapped to a click, so the
/// "keep running" result can be ignored.
fn press(app: &mut App, code: KeyCode) {
    let _ = crate::event::handle_key(app, key(code));
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

/// Apply one mouse event.
pub fn handle(app: &mut App, event: MouseEvent) {
    let dialog_open = app.modal != ActiveModal::None;
    match event.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            // Read-only overlays close on any click.
            if matches!(app.modal, ActiveModal::Help | ActiveModal::MetricsDetail) {
                press(app, KeyCode::Esc);
                return;
            }
            if let Some(target) = app.hits.at(event.column, event.row, dialog_open) {
                let double = app.clicks.click(target);
                click(app, target, double);
            }
        }
        MouseEventKind::ScrollUp => scroll(app, event, dialog_open, -1),
        MouseEventKind::ScrollDown => scroll(app, event, dialog_open, 1),
        _ => {}
    }
}

fn focus(app: &mut App, panel: FocusPanel) {
    // While a message is being edited inline, keys belong to the editor;
    // moving focus away would strand the edit.
    if !app.detail_editing {
        app.focus = panel;
    }
}

fn click(app: &mut App, target: Target, double: bool) {
    match target {
        Target::Panel(panel) => focus(app, panel),
        Target::TreeRow { index, on_toggle } => {
            focus(app, FocusPanel::Tree);
            if app.focus != FocusPanel::Tree || index >= app.flat_nodes.len() {
                return;
            }
            app.tree_selected = index;
            let expandable = app.flat_nodes[index].has_children;
            if expandable && (on_toggle || double) {
                app.toggle_expand();
            }
        }
        Target::MessageRow(index) => {
            focus(app, FocusPanel::Messages);
            if app.focus != FocusPanel::Messages || app.detail_editing {
                return;
            }
            if index < app.message_filtered_indices.len() {
                app.message_selected = index;
                if double {
                    press(app, KeyCode::Enter);
                }
            }
        }
        Target::MessageTab(tab) => {
            focus(app, FocusPanel::Messages);
            if app.focus == FocusPanel::Messages && !app.detail_editing {
                let code = match tab {
                    MessageTab::Messages => KeyCode::Char('1'),
                    MessageTab::DeadLetter => KeyCode::Char('2'),
                };
                press(app, code);
            }
        }
        Target::DialogListRow(index) => {
            select_dialog_row(app, index);
            if double {
                press(app, KeyCode::Enter);
            }
        }
        Target::FormField(index) => {
            if let Some((_, value)) = app.input_fields.get(index) {
                app.form_cursor = value.len();
                app.input_field_index = index;
            }
        }
        Target::Key(code) => press(app, code),
    }
}

/// Move the selection of whichever list the open dialog shows.
fn select_dialog_row(app: &mut App, index: usize) {
    match app.modal {
        ActiveModal::ConnectionList | ActiveModal::CopySelectConnection => {
            app.input_field_index = index;
        }
        ActiveModal::NamespaceDiscovery { .. } => app.namespace_list_state = index,
        ActiveModal::CopySelectEntity => {
            app.copy_entity_selected = index;
            app.copy_entity_list_state.select(Some(index));
        }
        _ => {}
    }
}

/// Scroll whatever is under the pointer by one step in `direction`.
fn scroll(app: &mut App, event: MouseEvent, dialog_open: bool, direction: i32) {
    let code = if direction < 0 {
        KeyCode::Up
    } else {
        KeyCode::Down
    };

    if dialog_open {
        // Lists and the multi-line body editor already scroll with ↑/↓.
        let scrollable = matches!(
            app.modal,
            ActiveModal::ConnectionList
                | ActiveModal::CopySelectConnection
                | ActiveModal::CopySelectEntity
                | ActiveModal::NamespaceDiscovery { .. }
                | ActiveModal::Help
        );
        if scrollable {
            press(app, code);
        }
        return;
    }

    let panel = match app.hits.at(event.column, event.row, false) {
        Some(Target::Panel(p)) => Some(p),
        Some(Target::TreeRow { .. }) => Some(FocusPanel::Tree),
        Some(Target::MessageRow(_) | Target::MessageTab(_)) => Some(FocusPanel::Messages),
        _ => None,
    };
    let Some(panel) = panel else {
        return;
    };
    focus(app, panel);
    if app.focus != panel {
        return;
    }

    match panel {
        FocusPanel::Tree => press(app, code),
        FocusPanel::Messages if app.detail_editing => {}
        // In the message detail view ↑/↓ scroll the body.
        FocusPanel::Messages => press(app, code),
        FocusPanel::Detail => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: u16, y: u16, w: u16, h: u16) -> Rect {
        Rect {
            x,
            y,
            width: w,
            height: h,
        }
    }

    #[test]
    fn topmost_target_wins() {
        let mut map = HitMap::default();
        map.add(rect(0, 0, 10, 10), Target::Panel(FocusPanel::Tree));
        map.add_row(
            rect(1, 1, 8, 5),
            2,
            Target::TreeRow {
                index: 4,
                on_toggle: false,
            },
        );
        assert_eq!(
            map.at(3, 3, false),
            Some(Target::TreeRow {
                index: 4,
                on_toggle: false
            })
        );
        assert_eq!(map.at(3, 5, false), Some(Target::Panel(FocusPanel::Tree)));
        assert_eq!(map.at(20, 20, false), None);
    }

    #[test]
    fn open_dialog_blocks_clicks_on_panels_beneath() {
        let mut map = HitMap::default();
        map.add(rect(0, 0, 80, 24), Target::Panel(FocusPanel::Tree));
        map.begin_dialog();
        map.add(rect(30, 10, 10, 1), Target::Key(KeyCode::Char('y')));
        assert_eq!(map.at(35, 10, true), Some(Target::Key(KeyCode::Char('y'))));
        assert_eq!(map.at(2, 2, true), None, "panel behind dialog is inert");
    }

    #[test]
    fn rows_outside_the_area_are_not_registered() {
        let mut map = HitMap::default();
        map.add_row(rect(0, 0, 10, 3), 3, Target::MessageRow(9));
        assert_eq!(map.at(1, 3, false), None);
    }

    #[test]
    fn second_click_on_same_target_is_a_double_click() {
        let mut clicks = ClickTracker::default();
        let t = Target::MessageRow(2);
        assert!(!clicks.click(t));
        assert!(clicks.click(t));
        assert!(!clicks.click(t), "third click starts a new sequence");
        assert!(!clicks.click(Target::MessageRow(3)));
    }
}
