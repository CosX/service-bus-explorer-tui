mod app;
mod bg_events;
mod bulk_ops;
mod client;
mod commands;
mod config;
mod event;
mod event_modal;
mod loaders;
mod mouse;
mod ui;

use std::io;
use std::time::{Duration, Instant};

use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;

use app::{App, MetricsAvailability};

/// Put the terminal back into its normal state. Safe to call more than once.
fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
    let _ = execute!(io::stdout(), crossterm::cursor::Show);
}

/// Restore the terminal before a panic message is printed; otherwise a panic
/// leaves the shell in raw mode on the alternate screen.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        previous(info);
    }));
}

/// Start file logging when enabled in settings. The returned guard flushes
/// buffered log lines on drop, so it must live as long as the app.
fn init_logging(enabled: bool) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    if !enabled {
        return None;
    }
    let dir = config::AppConfig::config_path()
        .parent()
        .map(std::path::Path::to_path_buf)?;
    let appender = tracing_appender::rolling::daily(dir, "sb-explorer.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_env_filter(filter)
        .try_init()
        .ok()?;
    Some(guard)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    install_panic_hook();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal).await;

    restore_terminal();

    if let Err(e) = result {
        eprintln!("Error: {}", e);
    }

    Ok(())
}

/// How long the tree selection must stay put before the detail pane and
/// metrics are loaded for it.
const DETAIL_DEBOUNCE: Duration = Duration::from_millis(250);

/// Debounce state for the tree selection driving the detail pane.
#[derive(Debug, Clone, Copy)]
enum DetailSelection {
    /// Detail has been dispatched for this index.
    Settled(usize),
    /// Selection moved to this index; waiting for it to stop moving.
    Pending { index: usize, since: Instant },
}

impl DetailSelection {
    /// Record the currently selected index, restarting the debounce window
    /// whenever it differs from what is settled or already pending.
    fn observe(self, selected: usize) -> Self {
        let current = match self {
            DetailSelection::Settled(index) => index,
            DetailSelection::Pending { index, .. } => index,
        };
        if current == selected {
            self
        } else {
            DetailSelection::Pending {
                index: selected,
                since: Instant::now(),
            }
        }
    }

    /// Returns true exactly once, when a pending selection has been stable for
    /// `DETAIL_DEBOUNCE`; transitions to `Settled` so it does not fire again.
    fn take_due(&mut self) -> bool {
        match *self {
            DetailSelection::Pending { index, since } if since.elapsed() >= DETAIL_DEBOUNCE => {
                *self = DetailSelection::Settled(index);
                true
            }
            _ => false,
        }
    }

    fn is_settled_on(&self, selected: usize) -> bool {
        matches!(*self, DetailSelection::Settled(index) if index == selected)
    }
}

async fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> anyhow::Result<()> {
    let mut app = App::new();
    let _log_guard = init_logging(app.config.settings.log_to_file);
    if let Some(warning) = app.config_warning.take() {
        app.set_error(warning);
    }
    let mut needs_refresh = false;
    let mut detail_selection = DetailSelection::Settled(usize::MAX);

    loop {
        terminal.draw(|frame| {
            ui::layout::render(frame, &mut app);
        })?;

        if !event::handle_events(&mut app)? || !app.running {
            break;
        }

        // ──────── Results from background tasks ────────
        while let Ok(event) = app.bg_rx.try_recv() {
            if bg_events::handle(&mut app, event) {
                needs_refresh = true;
            }
        }

        // Reset loading state if disconnected while an operation was queued.
        if app.management.is_none() && app.loading {
            app.loading = false;
            app.bg_running = false;
        }

        // Connection just established — load the tree.
        if app.management.is_some() && app.tree.is_none() && !app.loading {
            needs_refresh = true;
            app.last_refresh = Some(Instant::now());
        }

        // Auto-refresh timer.
        if app.auto_refresh_enabled
            && !app.bg_running
            && !app.loading
            && app.management.is_some()
            && app.config.settings.auto_refresh_secs > 0
            && app.last_refresh.is_some_and(|last| {
                last.elapsed().as_secs() >= app.config.settings.auto_refresh_secs
            })
        {
            needs_refresh = true;
        }

        if needs_refresh {
            loaders::spawn_tree_refresh(&mut app);
            needs_refresh = false;
        }

        // Load detail once the selection has settled (debounced, so scrolling
        // through the tree does not fire a request per entity).
        if !app.flat_nodes.is_empty() {
            detail_selection = detail_selection.observe(app.tree_selected);
        }
        if detail_selection.take_due() {
            loaders::spawn_detail_load(&mut app);
        }
        let selection_settled = detail_selection.is_settled_on(app.tree_selected);
        loaders::refetch_metrics_if_needed(&mut app, selection_settled);

        // Retry ARM resource ID resolution (m pressed after a failure).
        if matches!(app.metrics, MetricsAvailability::RetryRequested) {
            loaders::spawn_resource_id_resolve(&mut app, true);
        }

        // ──────── User-requested operations ────────
        if let Some(command) = app.pending_command.take() {
            commands::dispatch(&mut app, command);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{DetailSelection, DETAIL_DEBOUNCE};
    use std::time::Instant;

    /// A pending selection whose debounce window has already elapsed.
    fn expired_pending(index: usize) -> DetailSelection {
        DetailSelection::Pending {
            index,
            since: Instant::now() - DETAIL_DEBOUNCE,
        }
    }

    #[test]
    fn settled_selection_stays_settled_while_unchanged() {
        let mut state = DetailSelection::Settled(3).observe(3);
        assert!(state.is_settled_on(3));
        assert!(!state.take_due());
    }

    #[test]
    fn moving_selection_becomes_pending() {
        let mut state = DetailSelection::Settled(3).observe(4);
        assert!(!state.is_settled_on(4));
        assert!(
            !state.take_due(),
            "must not fire before the debounce elapses"
        );
    }

    #[test]
    fn scrolling_restarts_the_debounce_window() {
        // An expired window for entity 4 is discarded when the user keeps scrolling.
        let mut state = expired_pending(4).observe(5);
        assert!(!state.take_due());
    }

    #[test]
    fn settled_selection_fires_once_after_debounce() {
        let mut state = expired_pending(4);
        assert!(state.take_due());
        assert!(state.is_settled_on(4));
        assert!(
            !state.take_due(),
            "must not fire twice for the same selection"
        );
    }
}
