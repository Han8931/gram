//! gram — a Vim-flavoured spaced-repetition vocabulary trainer for the terminal.

mod app;
mod complete;
mod model;
mod search;
mod speech;
mod store;
mod textinput;
mod ui;
mod util;

use std::time::Duration;

use anyhow::{Context, Result, bail};
use chrono::Local;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event};

use app::App;

const USAGE: &str = "\
gram — spaced-repetition vocabulary study in the terminal

USAGE
    gram                            open the study interface
    gram import <file> [deck]       import a CSV/TSV deck, then exit
    gram export <deck> <file>       write a deck out as CSV
    gram decks                      list decks and what is due
    gram path                       print the collection file path

The collection lives at $GRAM_DATA, else $XDG_DATA_HOME/gram/collection.json,
else ~/.local/share/gram/collection.json.

Press ? inside the program for keybindings.
";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    match args.first().map(String::as_str) {
        None => run_tui(),
        Some("-h" | "--help" | "help") => {
            print!("{USAGE}");
            Ok(())
        }
        Some("-V" | "--version") => {
            println!("gram {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("path") => {
            println!("{}", store::data_path()?.display());
            Ok(())
        }
        Some("decks") => cmd_decks(),
        Some("import") => cmd_import(&args[1..]),
        Some("export") => cmd_export(&args[1..]),
        Some(other) => {
            bail!("unknown command: {other}\n\n{USAGE}");
        }
    }
}

fn run_tui() -> Result<()> {
    let path = store::data_path()?;
    let db = store::load_from(&path)?;
    let mut app = App::new(db, path);
    app.refresh_rows();
    ratatui::run(|terminal| event_loop(terminal, &mut app))
}

/// How long to sit waiting for a key before redrawing anyway. A session left
/// open across midnight would otherwise show yesterday's due counts until
/// the next key arrived.
const IDLE_REDRAW: Duration = Duration::from_secs(30);

fn event_loop(terminal: &mut DefaultTerminal, app: &mut App) -> Result<()> {
    while !app.quit {
        app.refresh_today();
        terminal.draw(|frame| ui::render(frame, app))?;
        if !event::poll(IDLE_REDRAW)? {
            continue;
        }
        // A resize needs nothing more than the redraw at the top of the loop.
        if let Event::Key(key) = event::read()? {
            app.on_key(key)?;
        }
    }
    if app.dirty {
        app.save()?;
    }
    Ok(())
}

fn cmd_decks() -> Result<()> {
    let db = store::load()?;
    if db.decks.is_empty() {
        println!("no decks yet — try: gram import <file.csv>");
        return Ok(());
    }
    let today = Local::now().date_naive();
    println!(
        "{:<30} {:>6} {:>6} {:>6} {:>8}  STATUS",
        "DECK", "NEW", "LEARN", "DUE", "TOTAL"
    );
    for deck in &db.decks {
        let counts = deck.counts(today);
        println!(
            "{:<30} {:>6} {:>6} {:>6} {:>8}  {}",
            deck.name,
            counts.new,
            counts.learning,
            counts.due,
            counts.total,
            counts.status().label()
        );
    }
    Ok(())
}

fn cmd_import(args: &[String]) -> Result<()> {
    let Some(path_arg) = args.first() else {
        bail!("usage: gram import <file.csv> [deck name]");
    };
    let path = store::expand_tilde(path_arg)?;
    let name = if args.len() > 1 {
        args[1..].join(" ")
    } else {
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("imported")
            .replace(['_', '-'], " ")
    };

    let mut db = store::load()?;
    let index = match db.deck_index(&name) {
        Some(index) => index,
        None => db.add_deck(name.clone()),
    };

    let today = Local::now().date_naive();
    let mut next = db.next_card_id();
    let mut deck = std::mem::replace(&mut db.decks[index], model::Deck::new(0, String::new()));
    let report = store::import_csv(
        &mut deck,
        &mut || {
            next += 1;
            next - 1
        },
        &path,
        today,
    )
    .with_context(|| format!("importing {}", path.display()))?;
    db.decks[index] = deck;
    db.reseat_ids();
    store::save(&db)?;

    println!(
        "{name}: {} added, {} updated, {} skipped ({} cards total)",
        report.added,
        report.updated,
        report.skipped,
        db.decks[index].cards.len()
    );
    Ok(())
}

fn cmd_export(args: &[String]) -> Result<()> {
    if args.len() < 2 {
        bail!("usage: gram export <deck> <file.csv>");
    }
    let db = store::load()?;
    let Some(index) = db.deck_index(&args[0]) else {
        bail!("no deck named {:?}", args[0]);
    };
    let path = store::expand_tilde(&args[1])?;
    let count = store::export_csv(&db.decks[index], &path)?;
    println!("exported {count} cards to {}", path.display());
    Ok(())
}
