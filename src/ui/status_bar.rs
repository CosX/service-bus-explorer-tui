use ratatui::prelude::*;
use ratatui::widgets::*;
use ratatui::Frame;

use crate::app::App;

pub fn render_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let style = if app.status_is_error {
        Style::default().bg(Color::Red).fg(Color::White)
    } else {
        Style::default().bg(Color::DarkGray).fg(Color::White)
    };

    let left = Span::styled(format!(" {} ", app.status_message), style);

    let right_text = match app.focus {
        crate::app::FocusPanel::Tree => "Tree",
        crate::app::FocusPanel::Detail => "Detail",
        crate::app::FocusPanel::Messages => "Messages",
    };
    let right = Span::styled(
        format!(" {} | ? Help ", right_text),
        Style::default().bg(Color::DarkGray).fg(Color::Gray),
    );

    let bar = Line::from(vec![
        left,
        Span::styled(
            " ".repeat(
                area.width
                    .saturating_sub(app.status_message.len() as u16 + right_text.len() as u16 + 12)
                    as usize,
            ),
            Style::default().bg(Color::DarkGray),
        ),
        right,
    ]);

    frame.render_widget(Paragraph::new(bar), area);
}
