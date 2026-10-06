//! Persistence: the JSON collection file plus CSV/TSV import and export.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result, anyhow, bail};
use chrono::{Local, NaiveDate};

use crate::model::{Card, Db, Deck};

/// Location of the collection file.
///
/// `$GRAM_DATA` wins, then `$XDG_DATA_HOME/gram`, then `~/.local/share/gram`.
pub fn data_path() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("GRAM_DATA")
        && !path.is_empty()
    {
        return Ok(PathBuf::from(path));
    }
    let base = match std::env::var("XDG_DATA_HOME") {
        Ok(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => {
            let home = std::env::var("HOME").context("neither $GRAM_DATA nor $HOME is set")?;
            PathBuf::from(home).join(".local/share")
        }
    };
    Ok(base.join("gram").join("collection.json"))
}

pub fn load() -> Result<Db> {
    let path = data_path()?;
    load_from(&path)
}

pub fn load_from(path: &Path) -> Result<Db> {
    if !path.exists() {
        return Ok(Db::default());
    }
    let text = fs::read_to_string(path)
        .with_context(|| format!("reading collection {}", path.display()))?;
    if text.trim().is_empty() {
        return Ok(Db::default());
    }
    let mut db: Db = serde_json::from_str(&text)
        .with_context(|| format!("parsing collection {}", path.display()))?;
    db.reseat_ids();
    db.migrate();
    Ok(db)
}

pub fn save(db: &Db) -> Result<()> {
    let path = data_path()?;
    save_to(db, &path)
}

/// When the collection was last written, for spotting another instance
/// writing underneath us. `None` when there is no collection yet.
pub fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|data| data.modified()).ok()
}

/// Write via a temp file and rename, so an interrupted save can't truncate
/// the collection.
pub fn save_to(db: &Db, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(db)?;
    // The temp file carries the writer's pid: a shared name means two gram
    // instances saving at once can rename each other's half-written file over
    // the collection.
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    fs::write(&tmp, json).with_context(|| format!("writing {}", tmp.display()))?;
    if let Err(err) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(err).with_context(|| format!("replacing {}", path.display()));
    }
    Ok(())
}

/// Copy the collection aside, under a name that says what it is, and return
/// where it went. Used when the file on disk turns out to be someone else's
/// newer version: overwriting it is still the right call — this copy of the
/// collection is the one being studied — but it should not be unrecoverable.
pub fn keep_conflicting_copy(path: &Path) -> Result<PathBuf> {
    let stamp = Local::now().format("%Y%m%d-%H%M%S");
    let aside = path.with_extension(format!("conflict-{stamp}.json"));
    fs::copy(path, &aside)
        .with_context(|| format!("copying {} to {}", path.display(), aside.display()))?;
    Ok(aside)
}

/// Columns we recognise, by header name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Column {
    Term,
    Pron,
    Pos,
    Definition,
    Example,
    Notes,
    Ignore,
}

fn classify(header: &str) -> Column {
    let h = header.trim().to_ascii_lowercase();
    let h = h.trim_matches(|c: char| !c.is_ascii_alphanumeric());
    match h {
        "term" | "word" | "front" | "headword" | "vocab" | "vocabulary" => Column::Term,
        "pron" | "pronunciation" | "phonetic" | "ipa" | "say" => Column::Pron,
        "pos" | "part" | "partofspeech" | "speech" | "type" => Column::Pos,
        "definition" | "def" | "meaning" | "back" | "translation" | "gloss" => Column::Definition,
        "example" | "examples" | "sentence" | "sentences" | "usage" | "context" => Column::Example,
        "notes" | "note" | "extra" | "comment" | "related" => Column::Notes,
        _ => Column::Ignore,
    }
}

/// Positional layout used when the file has no recognisable header row.
fn positional(width: usize) -> Vec<Column> {
    match width {
        0 => vec![],
        1 => vec![Column::Term],
        2 => vec![Column::Term, Column::Definition],
        3 => vec![Column::Term, Column::Definition, Column::Example],
        4 => vec![
            Column::Term,
            Column::Pos,
            Column::Definition,
            Column::Example,
        ],
        _ => {
            let mut cols = vec![
                Column::Term,
                Column::Pron,
                Column::Pos,
                Column::Definition,
                Column::Example,
                Column::Notes,
            ];
            cols.truncate(width);
            cols.resize(width, Column::Ignore);
            cols
        }
    }
}

fn delimiter_for(path: &Path, first_line: &str) -> u8 {
    match path.extension().and_then(|e| e.to_str()) {
        Some("tsv") | Some("tab") => b'\t',
        Some("csv") => b',',
        // Unknown extension: guess from whichever separator is more common.
        _ => {
            if first_line.matches('\t').count() > first_line.matches(',').count() {
                b'\t'
            } else {
                b','
            }
        }
    }
}

pub struct ImportReport {
    pub added: usize,
    pub updated: usize,
    pub skipped: usize,
}

/// Import a delimited file into `deck`. Existing terms are updated in place
/// rather than duplicated, so re-importing a corrected file is safe.
pub fn import_csv(
    deck: &mut Deck,
    db_next_id: &mut dyn FnMut() -> u64,
    path: &Path,
    today: NaiveDate,
) -> Result<ImportReport> {
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let first_line = text.lines().next().unwrap_or_default();
    let delimiter = delimiter_for(path, first_line);

    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .flexible(true)
        .has_headers(false)
        .from_reader(text.as_bytes());

    let mut records = reader.records();
    let first = match records.next() {
        Some(record) => record.context("reading header row")?,
        None => bail!("{} is empty", path.display()),
    };

    // Treat row one as a header only if at least two cells are known names.
    let named: Vec<Column> = first.iter().map(classify).collect();
    let recognised = named.iter().filter(|c| **c != Column::Ignore).count();
    let has_header = recognised >= 2 && named.contains(&Column::Term);

    let columns = if has_header {
        named
    } else {
        positional(first.len())
    };

    let mut report = ImportReport {
        added: 0,
        updated: 0,
        skipped: 0,
    };

    let mut ingest = |record: &csv::StringRecord, report: &mut ImportReport| {
        let mut fields = [""; 6];
        for (i, value) in record.iter().enumerate() {
            let slot = match columns.get(i) {
                Some(Column::Term) => 0,
                Some(Column::Pron) => 1,
                Some(Column::Pos) => 2,
                Some(Column::Definition) => 3,
                Some(Column::Example) => 4,
                Some(Column::Notes) => 5,
                _ => continue,
            };
            fields[slot] = value.trim();
        }
        let [term, pron, pos, definition, example, notes] = fields;
        if term.is_empty() || definition.is_empty() {
            report.skipped += 1;
            return;
        }

        if let Some(existing) = deck
            .cards
            .iter_mut()
            .find(|c| c.term.eq_ignore_ascii_case(term))
        {
            // Refresh content, leave scheduling untouched.
            existing.pron = pron.to_string();
            existing.pos = pos.to_string();
            existing.definition = definition.to_string();
            existing.example = example.to_string();
            existing.notes = notes.to_string();
            report.updated += 1;
        } else {
            let mut card = Card::new(
                db_next_id(),
                term.to_string(),
                definition.to_string(),
                today,
            );
            card.pron = pron.to_string();
            card.pos = pos.to_string();
            card.example = example.to_string();
            card.notes = notes.to_string();
            deck.cards.push(card);
            report.added += 1;
        }
    };

    if !has_header {
        ingest(&first, &mut report);
    }
    for record in records {
        let record = record.context("reading row")?;
        ingest(&record, &mut report);
    }

    Ok(report)
}

pub fn export_csv(deck: &Deck, path: &Path) -> Result<usize> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).ok();
    }
    let mut writer =
        csv::Writer::from_path(path).with_context(|| format!("writing {}", path.display()))?;
    writer.write_record(["term", "pron", "pos", "definition", "example", "notes"])?;
    for card in &deck.cards {
        writer.write_record([
            &card.term,
            &card.pron,
            &card.pos,
            &card.definition,
            &card.example,
            &card.notes,
        ])?;
    }
    writer.flush()?;
    Ok(deck.cards.len())
}

/// Expand a leading `~` so `:import ~/decks/x.csv` works.
pub fn expand_tilde(input: &str) -> Result<PathBuf> {
    if let Some(rest) = input.strip_prefix("~/") {
        let home = std::env::var("HOME").map_err(|_| anyhow!("$HOME is not set"))?;
        Ok(PathBuf::from(home).join(rest))
    } else if input == "~" {
        Ok(PathBuf::from(
            std::env::var("HOME").map_err(|_| anyhow!("$HOME is not set"))?,
        ))
    } else {
        Ok(PathBuf::from(input))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()
    }

    fn import_str(contents: &str, name: &str) -> (Deck, ImportReport) {
        let dir = std::env::temp_dir().join(format!("gram-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        fs::write(&path, contents).unwrap();

        let mut deck = Deck::new(1, "test".into());
        let mut id = 0u64;
        let report = import_csv(
            &mut deck,
            &mut || {
                id += 1;
                id
            },
            &path,
            today(),
        )
        .unwrap();
        fs::remove_file(&path).ok();
        (deck, report)
    }

    /// A deck survives import, save, reload and export unchanged — the whole
    /// path a collection takes between sessions, in one test.
    #[test]
    fn a_collection_survives_the_round_trip() {
        let dir = std::env::temp_dir().join(format!("gram-roundtrip-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let source = dir.join("in.csv");
        fs::write(
            &source,
            "term,pron,pos,definition,example,notes\n\
             abash,uh BASH,v,to embarrass,Meredith felt *abashed*.,a note\n\
             한국어,han guk eo,n,the Korean language,\"한국어를 공부한다.\",두 줄\n",
        )
        .unwrap();

        // Import.
        let mut db = Db::default();
        let index = db.add_deck("round trip".into());
        let mut next = db.next_card_id();
        let mut deck = std::mem::replace(&mut db.decks[index], Deck::new(0, String::new()));
        let report = import_csv(
            &mut deck,
            &mut || {
                next += 1;
                next - 1
            },
            &source,
            today(),
        )
        .unwrap();
        db.decks[index] = deck;
        db.reseat_ids();
        assert_eq!((report.added, report.skipped), (2, 0));

        // Grade one card so there is scheduling state worth preserving.
        db.decks[index].cards[0].grade(crate::model::Grade::Good, today());

        // Save, and reload into a fresh Db.
        let collection = dir.join("collection.json");
        save_to(&db, &collection).unwrap();
        let reloaded = load_from(&collection).unwrap();

        assert_eq!(reloaded.decks.len(), 1);
        let after = &reloaded.decks[0];
        assert_eq!(after.cards.len(), 2);
        assert_eq!(after.cards[0].term, "abash");
        assert_eq!(after.cards[0].state, crate::model::CardState::Review);
        assert_eq!(after.cards[0].due, db.decks[index].cards[0].due);
        assert_eq!(after.cards[1].term, "한국어", "non-ASCII makes it back");
        assert_eq!(after.cards[1].definition, "the Korean language");

        // Export, and re-import the export: same cards, nothing duplicated.
        let out = dir.join("out.csv");
        assert_eq!(export_csv(after, &out).unwrap(), 2);
        let mut again = after.clone();
        let mut id = 1000u64;
        let report = import_csv(
            &mut again,
            &mut || {
                id += 1;
                id
            },
            &out,
            today(),
        )
        .unwrap();
        assert_eq!(
            (report.added, report.updated),
            (0, 2),
            "an export re-imports onto the same cards"
        );
        assert_eq!(again.cards.len(), 2);
        assert_eq!(
            again.cards[0].due, after.cards[0].due,
            "and leaves their scheduling alone"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn saving_leaves_no_temp_file_behind() {
        let dir = std::env::temp_dir().join(format!("gram-tmpfile-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let collection = dir.join("collection.json");

        save_to(&Db::default(), &collection).unwrap();
        let left: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, vec!["collection.json".to_string()]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn imports_named_headers_in_any_order() {
        let (deck, report) = import_str("definition,term,pos\nto embarrass,abash,v\n", "named.csv");
        assert_eq!(report.added, 1);
        assert_eq!(deck.cards[0].term, "abash");
        assert_eq!(deck.cards[0].definition, "to embarrass");
        assert_eq!(deck.cards[0].pos, "v");
    }

    #[test]
    fn imports_headerless_two_column_file() {
        let (deck, report) = import_str("abash,to embarrass\nabate,to subside\n", "plain.csv");
        assert_eq!(report.added, 2);
        assert_eq!(deck.cards[1].term, "abate");
    }

    #[test]
    fn reimport_updates_instead_of_duplicating() {
        let dir = std::env::temp_dir().join(format!("gram-reimport-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("d.csv");

        let mut deck = Deck::new(1, "test".into());
        let mut id = 0u64;
        let mut next = || {
            id += 1;
            id
        };

        fs::write(&path, "term,definition\nabash,to embarrass\n").unwrap();
        import_csv(&mut deck, &mut next, &path, today()).unwrap();
        deck.cards[0].interval = 30; // pretend it has been studied

        fs::write(&path, "term,definition\nabash,to make ashamed\n").unwrap();
        let report = import_csv(&mut deck, &mut next, &path, today()).unwrap();

        assert_eq!(deck.cards.len(), 1);
        assert_eq!(report.updated, 1);
        assert_eq!(deck.cards[0].definition, "to make ashamed");
        assert_eq!(
            deck.cards[0].interval, 30,
            "scheduling must survive reimport"
        );
        fs::remove_file(&path).ok();
    }

    #[test]
    fn skips_rows_without_a_definition() {
        let (deck, report) = import_str(
            "term,definition\nabash,\n,to subside\nabate,to subside\n",
            "gap.csv",
        );
        assert_eq!(report.added, 1);
        assert_eq!(report.skipped, 2);
        assert_eq!(deck.cards.len(), 1);
    }

    #[test]
    fn detects_tabs_in_tsv() {
        let (deck, report) = import_str("term\tdefinition\nabash\tto embarrass\n", "d.tsv");
        assert_eq!(report.added, 1);
        assert_eq!(deck.cards[0].definition, "to embarrass");
    }
}
