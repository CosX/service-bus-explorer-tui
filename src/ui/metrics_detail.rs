use ratatui::prelude::*;
use ratatui::symbols::Marker;
use ratatui::widgets::*;
use ratatui::Frame;

use crate::app::App;

pub fn render_metrics_detail(frame: &mut Frame, app: &App) {
    let area = centered_rect(80, 85, frame.area());
    frame.render_widget(Clear, area);

    let metrics = match &app.entity_metrics {
        Some(m) => m,
        None => {
            let block = Block::default()
                .title(" Metrics Detail (Esc to close) ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan));
            let msg = Paragraph::new("No metrics data available")
                .style(Style::default().fg(Color::DarkGray))
                .block(block);
            frame.render_widget(msg, area);
            return;
        }
    };

    let label = app.metrics_window.label();
    let title = format!(
        " Metrics: {} ({}) — M: cycle window, Esc: close ",
        metrics.entity_name, label
    );

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Collect series: (name, data_slice, color)
    let mut series: Vec<(&str, &[u64], Color)> = Vec::new();

    if !metrics.active_messages.is_empty() {
        series.push(("Active", &metrics.active_messages, Color::Green));
    }
    if !metrics.dead_letter_messages.is_empty() {
        series.push(("Dead-letter", &metrics.dead_letter_messages, Color::Red));
    }
    if !metrics.scheduled_messages.is_empty() {
        series.push(("Scheduled", &metrics.scheduled_messages, Color::Yellow));
    }
    if !metrics.incoming_messages.is_empty() {
        series.push(("Incoming", &metrics.incoming_messages, Color::Cyan));
    }
    if !metrics.outgoing_messages.is_empty() {
        series.push(("Outgoing", &metrics.outgoing_messages, Color::Blue));
    }

    if series.is_empty() {
        let msg = Paragraph::new("No metrics data available")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(msg, inner);
        return;
    }

    // Each chart gets an equal share of vertical space
    let constraints: Vec<Constraint> = series
        .iter()
        .map(|_| Constraint::Ratio(1, series.len() as u32))
        .collect();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    for (i, (name, data, color)) in series.iter().enumerate() {
        render_line_chart(frame, chunks[i], name, label, data, *color);
    }
}

/// Render a single metric as a braille dot line chart with y-axis labels.
fn render_line_chart(
    frame: &mut Frame,
    area: Rect,
    name: &str,
    window_label: &str,
    data: &[u64],
    color: Color,
) {
    if data.is_empty() {
        return;
    }

    let latest = data.last().copied().unwrap_or(0);
    let peak = data.iter().copied().max().unwrap_or(0);

    let title = if latest == peak {
        format!(" {} ({}) cur:{} ", name, window_label, latest)
    } else {
        format!(" {} ({}) cur:{} peak:{} ", name, window_label, latest, peak)
    };

    // Convert u64 data to (f64, f64) points for the Chart widget
    let points: Vec<(f64, f64)> = data
        .iter()
        .enumerate()
        .map(|(i, &v)| (i as f64, v as f64))
        .collect();

    let x_max = if points.len() > 1 {
        (points.len() - 1) as f64
    } else {
        1.0
    };
    // Add 10% headroom above peak so the line doesn't touch the top border
    let y_max = if peak == 0 { 10.0 } else { peak as f64 * 1.1 };

    let datasets = vec![Dataset::default()
        .marker(Marker::Braille)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(color))
        .data(&points)];

    let chart = Chart::new(datasets)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .x_axis(
            Axis::default()
                .bounds([0.0, x_max])
                .style(Style::default().fg(Color::DarkGray)),
        )
        .y_axis(
            Axis::default()
                .bounds([0.0, y_max])
                .labels(y_axis_labels(y_max))
                .style(Style::default().fg(Color::DarkGray)),
        );

    frame.render_widget(chart, area);
}

/// Generate 3 y-axis labels: 0, mid, max (formatted compactly).
fn y_axis_labels(y_max: f64) -> Vec<Line<'static>> {
    let max_val = y_max as u64;
    let mid_val = max_val / 2;
    vec![
        Line::from(Span::styled(
            "0",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            format_compact(mid_val),
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            format_compact(max_val),
            Style::default().fg(Color::DarkGray),
        )),
    ]
}

/// Format large numbers compactly (e.g. 1500 -> "1.5k", 2000000 -> "2M").
fn format_compact(v: u64) -> String {
    if v >= 1_000_000 {
        format!("{:.1}M", v as f64 / 1_000_000.0)
    } else if v >= 1_000 {
        format!("{:.1}k", v as f64 / 1_000.0)
    } else {
        v.to_string()
    }
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
