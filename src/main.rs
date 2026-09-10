mod app;
mod clipboard;
mod config;
mod docker;
mod events;
mod ui;

use std::io;

use anyhow::Result;
use clap::Parser;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "dox", version, about = "A focused Docker TUI for colima")]
struct Cli {
    /// Override the docker daemon endpoint (e.g. `unix:///path/to/sock` or `tcp://host:2375`).
    #[arg(long)]
    host: Option<String>,

    /// Override list refresh interval in milliseconds.
    #[arg(long)]
    refresh_ms: Option<u64>,

    /// Force-enable mouse capture (overrides `mouse = false` in config.toml).
    /// On by default: click to focus panels and rows, wheel scrolls whatever is
    /// under the pointer. Hold Shift to fall back to native terminal selection.
    #[arg(long)]
    mouse: bool,

    /// Force-disable mouse capture (overrides `mouse = true` in config.toml).
    #[arg(long)]
    no_mouse: bool,

    /// Smoke test: list containers and exit (no TUI).
    #[arg(long)]
    list_containers: bool,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let _guard = init_tracing();

    let mut cfg = config::load()?;
    if let Some(ms) = cli.refresh_ms {
        cfg.refresh_ms = ms;
    }
    if cli.mouse {
        cfg.mouse = true;
    }
    if cli.no_mouse {
        cfg.mouse = false;
    }

    let client = docker::connect(cli.host).await?;
    info!(version = %client.info.server_version, socket = %client.info.socket, "connected");

    if cli.list_containers {
        let rows = docker::containers::list(&client).await?;
        println!(
            "connected to docker {} via {}",
            client.info.server_version, client.info.socket
        );
        for r in rows {
            println!(
                "{:14}  {:30}  {:30}  {}",
                docker::containers::short(&r.id),
                truncate(&r.name, 30),
                truncate(&r.image, 30),
                r.state
            );
        }
        return Ok(());
    }

    run_tui(client, cfg).await
}

async fn run_tui(client: docker::DockerClient, cfg: config::Config) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    if cfg.mouse {
        execute!(stdout, EnableMouseCapture)?;
    }
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let clipboard: Box<dyn clipboard::Clipboard> = Box::new(clipboard::SystemClipboard);
    let mouse_enabled = cfg.mouse;
    let result = app::run(&mut terminal, client, cfg, clipboard).await;

    disable_raw_mode()?;
    if mouse_enabled {
        execute!(terminal.backend_mut(), DisableMouseCapture)?;
    }
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

fn init_tracing() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let dir = dirs::cache_dir()?.join("dox");
    if std::fs::create_dir_all(&dir).is_err() {
        return None;
    }
    let file = tracing_appender::rolling::never(dir, "dox.log");
    let (writer, guard) = tracing_appender::non_blocking(file);
    let filter = EnvFilter::try_from_env("DOX_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(false)
        .init();
    Some(guard)
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}
