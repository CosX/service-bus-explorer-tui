use ratatui::prelude::*;
use ratatui::widgets::*;
use ratatui::Frame;

pub fn render_help(frame: &mut Frame) {
    let area = centered_rect(60, 70, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Keyboard Shortcuts (press any key to close) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

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
            "  Connection",
            Style::default().fg(Color::Cyan).bold(),
        )]),
        Line::from("  c              Connect / Switch connection"),
        Line::from("  r / F5         Refresh entities"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  Entity Operations",
            Style::default().fg(Color::Cyan).bold(),
        )]),
        Line::from("  n              Create new entity"),
        Line::from("  x              Delete selected entity"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  Message Operations",
            Style::default().fg(Color::Cyan).bold(),
        )]),
        Line::from("  p              Peek messages (prompts for count)"),
        Line::from("  d              Peek dead-letter queue"),
        Line::from("  s              Send message"),
        Line::from("  P (shift)      Clear entity (delete all / resend DLQ)"),
        Line::from(Span::styled(
            "                 (on topics: operates across all subs)",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from("  1/2            Switch Messages/DLQ tab"),
        Line::from("  Enter          View message detail"),
        Line::from("  Esc            Close message detail"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  Bulk Operations (Messages panel)",
            Style::default().fg(Color::Cyan).bold(),
        )]),
        Line::from("  R (shift)      Resend peeked DLQ → main entity"),
        Line::from("  D (shift)      Bulk delete messages"),
        Line::from(Span::styled(
            "                 (on topics: fan-out across all subs)",
            Style::default().fg(Color::DarkGray),
        )),
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

    let paragraph = Paragraph::new(help_text).block(block);
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
