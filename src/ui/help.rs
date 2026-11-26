use ratatui::prelude::*;
use ratatui::widgets::*;
use ratatui::Frame;

use crate::app::App;

pub fn render_help(frame: &mut Frame, app: &mut App) {
    let area = centered_rect(60, 80, frame.area());
    frame.render_widget(Clear, area);

    let help_text = vec![
        Line::from(""),
        Line::from(vec![Span::styled(
            "  Navigation",
            Style::default().fg(Color::Cyan).bold(),
        )]),
        Line::from("  ↑/k, ↓/j      Move up/down"),
        Line::from("  ←/h, →/l       Collapse/Expand"),
        Line::from("  Tab/Shift+Tab  Switch panels"),
        Line::from("  Enter          Select/Expand"),
        Line::from("  g/G            First/Last item"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  Mouse",
            Style::default().fg(Color::Cyan).bold(),
        )]),
        Line::from("  Click          Focus panel / select row / press button"),
        Line::from("  Double-click   Open message, expand node, pick list item"),
        Line::from("  Click ▶/▼      Expand/Collapse"),
        Line::from("  Wheel          Scroll list or message body"),
        Line::from("  Shift+drag     Select text (Option+drag in some macOS terminals)"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  Connection",
            Style::default().fg(Color::Cyan).bold(),
        )]),
        Line::from("  c              Connect / Switch connection"),
        Line::from("  r / F5         Refresh entities"),
        Line::from("  t              Toggle auto-refresh timer"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  Entity Operations",
            Style::default().fg(Color::Cyan).bold(),
        )]),
        Line::from("  n              Create new entity"),
        Line::from("  x              Delete selected entity"),
        Line::from("  f              Edit selected subscription filter"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  Message Operations",
            Style::default().fg(Color::Cyan).bold(),
        )]),
        Line::from("  p              Peek messages (prompts for count)"),
        Line::from("  d              Peek dead-letter queue"),
        Line::from("  s              Send message"),
        Line::from("  m              Toggle Azure Monitor metrics"),
        Line::from("  M (shift)      Cycle metrics window (1h/6h/24h/7d)"),
        Line::from("  V (shift)      Open metrics detail overlay"),
        Line::from("  P (shift)      Clear entity, or all queues/topics on a folder"),
        Line::from(Span::styled(
            "                 (on topics: operates across all subs)",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from("  L (shift)      Purge ALL DLQs (entire namespace)"),
        Line::from("  /              Filter messages (type to search)"),
        Line::from("  1/2            Switch Messages/DLQ tab"),
        Line::from("  Enter          View message detail"),
        Line::from("  Esc            Close message detail"),
        Line::from("  f              Toggle raw/formatted body (JSON/XML)"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  Bulk Operations (Messages panel)",
            Style::default().fg(Color::Cyan).bold(),
        )]),
        Line::from("  R (shift)      Resend peeked DLQ → main entity"),
        Line::from("  D (shift)      Delete the peeked messages"),
        Line::from(Span::styled(
            "                 (on topics: fan-out across all subs)",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from("  x              Delete selected message"),
        Line::from("  e              Edit & resend (inline WYSIWYG)"),
        Line::from(vec![
            Span::styled("  C       ", Style::default().fg(Color::Yellow)),
            Span::raw("Copy message to different connection"),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  Editing (inline & forms)",
            Style::default().fg(Color::Cyan).bold(),
        )]),
        Line::from("  F2             Send / submit"),
        Line::from("  ←/→/Home/End   Move cursor in field"),
        Line::from("  Tab/↑↓         Navigate between fields"),
        Line::from("  Esc            Cancel editing"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  General",
            Style::default().fg(Color::Cyan).bold(),
        )]),
        Line::from("  ?              Show this help"),
        Line::from("  q / Ctrl+C     Quit"),
        Line::from(""),
    ];

    // Scroll within the box; the renderer is the one that knows what fits.
    let visible = area.height.saturating_sub(2);
    let total = help_text.len() as u16;
    app.help_max_scroll = total.saturating_sub(visible);
    app.help_scroll = app.help_scroll.min(app.help_max_scroll);

    let title = if app.help_max_scroll > 0 {
        format!(
            " Help ({}/{}) — ↑↓/wheel scroll · other key/click closes ",
            app.help_scroll + visible.min(total),
            total
        )
    } else {
        " Keyboard & Mouse (press any key or click to close) ".to_string()
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let paragraph = Paragraph::new(help_text)
        .block(block)
        .scroll((app.help_scroll, 0));
    frame.render_widget(paragraph, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
