use ratatui::prelude::*;
use ratatui::widgets::*;
use ratatui::Frame;

use crate::app::{ActiveModal, App};

use super::detail::render_detail;
use super::help::render_help;
use super::messages::render_messages;
use super::modals::render_modal;
use super::status_bar::render_status_bar;
use super::tree::render_tree;

pub fn render(frame: &mut Frame, app: &mut App) {
    let size = frame.area();

    // Main layout: [status bar] [body] [keyhints]
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // title bar
            Constraint::Min(10),   // body
            Constraint::Length(1), // status bar
        ])
        .split(size);

    // Title bar
    let title = if let Some(ref name) = app.connection_name {
        format!(" Service Bus Explorer — {} ", name)
    } else {
        " Service Bus Explorer — Not Connected ".to_string()
    };
    let title_bar =
        Paragraph::new(title).style(Style::default().bg(Color::Blue).fg(Color::White).bold());
    frame.render_widget(title_bar, outer[0]);

    // Body: [tree | detail+messages]
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(30), // tree
            Constraint::Percentage(70), // right side
        ])
        .split(outer[1]);

    // Right side: [detail | messages]
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(40), // detail
            Constraint::Percentage(60), // messages
        ])
        .split(body[1]);

    // Render panels
    render_tree(frame, app, body[0]);
    render_detail(frame, app, right[0]);
    render_messages(frame, app, right[1]);
    render_status_bar(frame, app, outer[2]);

    // Render modal overlay if active
    if app.modal != ActiveModal::None {
        render_modal(frame, app);
    }

    // Render help overlay
    if app.modal == ActiveModal::Help {
        render_help(frame);
    }
}
