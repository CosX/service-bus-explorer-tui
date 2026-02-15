use ratatui::prelude::*;
use ratatui::widgets::*;
use ratatui::Frame;

use crate::app::{App, DetailView, FocusPanel};

pub fn render_detail(frame: &mut Frame, app: &App, area: Rect) {
    let is_focused = app.focus == FocusPanel::Detail;
    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .title(" Properties ")
        .borders(Borders::ALL)
        .border_style(border_style);

    match &app.detail_view {
        DetailView::None => {
            let msg = Paragraph::new("Select an entity to view properties")
                .style(Style::default().fg(Color::DarkGray))
                .block(block);
            frame.render_widget(msg, area);
        }
        DetailView::Queue(desc, runtime) => {
            let mut rows = vec![
                make_row("Name", &desc.name),
                make_row("Status", desc.status.as_deref().unwrap_or("Active")),
                make_row(
                    "Lock Duration",
                    desc.lock_duration.as_deref().unwrap_or("-"),
                ),
                make_row("Max Size (MB)", &opt_i64(desc.max_size_in_megabytes)),
                make_row(
                    "Default TTL",
                    desc.default_message_time_to_live.as_deref().unwrap_or("-"),
                ),
                make_row("Max Delivery Count", &opt_i32(desc.max_delivery_count)),
                make_row("Requires Session", &opt_bool(desc.requires_session)),
                make_row("Partitioning", &opt_bool(desc.enable_partitioning)),
                make_row(
                    "DLQ on Expiry",
                    &opt_bool(desc.dead_lettering_on_message_expiration),
                ),
            ];

            if let Some(ref fwd) = desc.forward_to {
                rows.push(make_row("Forward To", fwd));
            }
            if let Some(ref fwd) = desc.forward_dead_lettered_messages_to {
                rows.push(make_row("Fwd DLQ To", fwd));
            }

            if let Some(rt) = runtime {
                rows.push(make_row("──────────", "──────────"));
                rows.push(make_row(
                    "Active Messages",
                    &rt.active_message_count.to_string(),
                ));
                rows.push(make_row(
                    "Dead-letter",
                    &rt.dead_letter_message_count.to_string(),
                ));
                rows.push(make_row(
                    "Scheduled",
                    &rt.scheduled_message_count.to_string(),
                ));
                rows.push(make_row("Size (bytes)", &rt.size_in_bytes.to_string()));
            }

            render_table(frame, area, block, rows);
        }
        DetailView::Topic(desc, runtime) => {
            let mut rows = vec![
                make_row("Name", &desc.name),
                make_row("Status", desc.status.as_deref().unwrap_or("Active")),
                make_row("Max Size (MB)", &opt_i64(desc.max_size_in_megabytes)),
                make_row(
                    "Default TTL",
                    desc.default_message_time_to_live.as_deref().unwrap_or("-"),
                ),
                make_row("Partitioning", &opt_bool(desc.enable_partitioning)),
            ];

            if let Some(rt) = runtime {
                rows.push(make_row("──────────", "──────────"));
                rows.push(make_row(
                    "Subscriptions",
                    &rt.subscription_count.to_string(),
                ));
                rows.push(make_row(
                    "Scheduled",
                    &rt.scheduled_message_count.to_string(),
                ));
                rows.push(make_row("Size (bytes)", &rt.size_in_bytes.to_string()));
            }

            render_table(frame, area, block, rows);
        }
        DetailView::Subscription(desc, runtime) => {
            let mut rows = vec![
                make_row("Name", &desc.name),
                make_row("Topic", &desc.topic_name),
                make_row("Status", desc.status.as_deref().unwrap_or("Active")),
                make_row(
                    "Lock Duration",
                    desc.lock_duration.as_deref().unwrap_or("-"),
                ),
                make_row(
                    "Default TTL",
                    desc.default_message_time_to_live.as_deref().unwrap_or("-"),
                ),
                make_row("Max Delivery Count", &opt_i32(desc.max_delivery_count)),
            ];

            if let Some(ref fwd) = desc.forward_to {
                rows.push(make_row("Forward To", fwd));
            }

            if let Some(rt) = runtime {
                rows.push(make_row("──────────", "──────────"));
                rows.push(make_row(
                    "Active Messages",
                    &rt.active_message_count.to_string(),
                ));
                rows.push(make_row(
                    "Dead-letter",
                    &rt.dead_letter_message_count.to_string(),
                ));
            }

            render_table(frame, area, block, rows);
        }
    }
}

fn make_row(label: &str, value: &str) -> Row<'static> {
    Row::new(vec![label.to_string(), value.to_string()])
}

fn opt_i64(v: Option<i64>) -> String {
    v.map(|v| v.to_string()).unwrap_or_else(|| "-".into())
}

fn opt_i32(v: Option<i32>) -> String {
    v.map(|v| v.to_string()).unwrap_or_else(|| "-".into())
}

fn opt_bool(v: Option<bool>) -> String {
    v.map(|v| v.to_string()).unwrap_or_else(|| "-".into())
}

fn render_table(frame: &mut Frame, area: Rect, block: Block, rows: Vec<Row>) {
    let table = Table::new(
        rows,
        [Constraint::Percentage(35), Constraint::Percentage(65)],
    )
    .block(block)
    .column_spacing(1);

    frame.render_widget(table, area);
}
