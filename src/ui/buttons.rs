//! Clickable button rows for dialogs and forms.

use crossterm::event::KeyCode;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::mouse::{HitMap, Target};

/// A button: its label, the key it presses, and its colour.
pub struct Button<'a> {
    pub label: &'a str,
    pub key: KeyCode,
    pub color: Color,
}

impl<'a> Button<'a> {
    pub const fn new(label: &'a str, key: KeyCode, color: Color) -> Self {
        Self { label, key, color }
    }
}

const GAP: u16 = 2;

/// Draw `buttons` as `[ label ]` on the first line of `area`, aligned as
/// asked, and register each one as a click target that presses its key.
pub fn render_buttons(
    frame: &mut Frame,
    area: Rect,
    hits: &mut HitMap,
    buttons: &[Button<'_>],
    alignment: Alignment,
) {
    if area.height == 0 || buttons.is_empty() {
        return;
    }
    let labels: Vec<String> = buttons.iter().map(|b| format!("[ {} ]", b.label)).collect();
    let widths: Vec<u16> = labels.iter().map(|l| l.chars().count() as u16).collect();
    let total: u16 = widths.iter().sum::<u16>() + GAP * (buttons.len() as u16 - 1);

    let start = match alignment {
        Alignment::Left => area.x,
        Alignment::Center => area.x + area.width.saturating_sub(total) / 2,
        Alignment::Right => area.x + area.width.saturating_sub(total),
    };
    let right_edge = area.x + area.width;

    let mut x = start;
    for ((button, label), width) in buttons.iter().zip(&labels).zip(&widths) {
        if x >= right_edge {
            break;
        }
        let rect = Rect {
            x,
            y: area.y,
            width: (*width).min(right_edge - x),
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(label.as_str())
                .style(Style::default().fg(Color::Black).bg(button.color).bold()),
            rect,
        );
        hits.add(rect, Target::Key(button.key));
        x = x.saturating_add(width + GAP);
    }
}
