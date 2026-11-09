mod app;
mod config;
mod gh;
mod ui;

use anyhow::Result;
use app::{App, Msg};
use ratatui::crossterm::event::{self, DisableMouseCapture, EnableMouseCapture};
use ratatui::crossterm::execute;
use std::io::stdout;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Notify};

#[tokio::main]
async fn main() -> Result<()> {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("pane {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if std::env::args().any(|a| a == "--help" || a == "-h") {
        println!(
            "pane: live terminal dashboard for open GitHub PRs and issues\n\nconfig: {}\nkeys: press ? inside the app\n--version: print the version\n--check: print the URL of every open PR and issue, then exit",
            config::path().display()
        );
        return Ok(());
    }
    let cfg = config::load()?;
    if std::env::args().any(|a| a == "--check") {
        let snap = gh::fetch_all(&cfg, |_, _| {}).await?;
        for i in snap
            .prs
            .iter()
            .map(|i| ("pr", i))
            .chain(snap.issues.iter().map(|i| ("issue", i)))
        {
            println!("{} {}", i.0, i.1.url);
        }
        return Ok(());
    }
    let (tx, mut rx) = mpsc::unbounded_channel::<Msg>();
    let refresh = Arc::new(Notify::new());

    let input_tx = tx.clone();
    std::thread::spawn(move || {
        while let Ok(ev) = event::read() {
            if input_tx.send(Msg::Term(ev)).is_err() {
                break;
            }
        }
    });

    let tick_tx = tx.clone();
    tokio::spawn(async move {
        let mut iv = tokio::time::interval(Duration::from_millis(100));
        loop {
            iv.tick().await;
            if tick_tx.send(Msg::Tick).is_err() {
                break;
            }
        }
    });

    let (list_tx, list_cfg, notify) = (tx.clone(), cfg.clone(), refresh.clone());
    tokio::spawn(async move {
        let mut iv = tokio::time::interval(Duration::from_secs(list_cfg.refresh_secs));
        iv.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = iv.tick() => {}
                _ = notify.notified() => iv.reset(),
            }
            let _ = list_tx.send(Msg::Loading);
            let progress_tx = list_tx.clone();
            let r = gh::fetch_all(&list_cfg, move |done, total| {
                let _ = progress_tx.send(Msg::Progress(done, total));
            })
            .await
            .map_err(|e| format!("{e:#}"));
            if list_tx.send(Msg::List(r)).is_err() {
                break;
            }
        }
    });

    let viewer_tx = tx.clone();
    tokio::spawn(async move {
        let r = gh::viewer().await.map_err(|e| format!("{e:#}"));
        let _ = viewer_tx.send(Msg::Viewer(r));
    });

    let mut app = App::new(cfg, tx, refresh);
    let mut terminal = ratatui::init();
    execute!(stdout(), EnableMouseCapture)?;

    let result = run(&mut terminal, &mut app, &mut rx).await;

    execute!(stdout(), DisableMouseCapture)?;
    ratatui::restore();
    result
}

async fn run(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    rx: &mut mpsc::UnboundedReceiver<Msg>,
) -> Result<()> {
    loop {
        terminal.draw(|f| ui::draw(f, app))?;
        let Some(msg) = rx.recv().await else {
            return Ok(());
        };
        app.update(msg);
        while let Ok(m) = rx.try_recv() {
            app.update(m);
        }
        if app.quit {
            return Ok(());
        }
    }
}
