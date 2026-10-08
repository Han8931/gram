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
///
/// Two conflicts in the same second must not land on the same name, or the
/// second copy destroys the thing the first one was saved to protect. The
/// stamp is only the starting point; a taken name gets a counter.
pub fn keep_conflicting_copy(path: &Path) -> Result<PathBuf> {
    let stamp = Local::now().format("%Y%m%d-%H%M%S");
    for attempt in 0..1000 {
        let aside = match attempt {
            0 => path.with_extension(format!("conflict-{stamp}.json")),
            n => path.with_extension(format!("conflict-{stamp}-{n}.json")),
        };
        // `create_new` fails rather than truncating when the name is taken, so
        // claiming the name and finding it free are one step — nothing between
        // them for another gram to slip into.
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&aside)
        {
            Ok(_) => {
                if let Err(err) = fs::copy(path, &aside) {
                    // The name is reserved by an empty file at this point.
                    // Leaving it would offer `:restore` an empty collection
                    // that looks like a real backup.
                    let _ = fs::remove_file(&aside);
                    return Err(err).with_context(|| {
                        format!("copying {} to {}", path.display(), aside.display())
                    });
                }
                return Ok(aside);
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => {
                return Err(err).with_context(|| format!("creating backup {}", aside.display()));
            }
        }
    }
    bail!(
        "cannot find an unused backup name beside {}",
        path.display()
    )
}

/// The part of a backup's name that says which backup it is. Every one of
/// them starts `collection.conflict-` and ends `.json`, so repeating that in a
/// status line only costs the room the stamp needs. Anything not shaped like a
/// backup keeps its file name.
pub fn backup_label(path: &Path) -> String {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    match name.rsplit_once(".json") {
        Some((before, "")) => match before.rsplit_once(".conflict-") {
            Some((_, stamp)) => stamp.to_string(),
            None => name.into_owned(),
        },
        _ => name.into_owned(),
    }
}

/// Where a backup falls in time: its stamp, then its counter. Sorting the
/// names as text would get this wrong — `-1` sorts before `.json`, putting the
/// second copy of a second ahead of the first.
fn backup_order(name: &str) -> (String, u32) {
    let stamped = name
        .rsplit_once(".json")
        .map_or(name, |(before, _)| before)
        .rsplit_once("conflict-")
        .map_or(name, |(_, after)| after);
    // `YYYYMMDD-HHMMSS`, optionally followed by `-<n>`.
    let mut parts = stamped.splitn(3, '-');
    let date = parts.next().unwrap_or_default();
    let time = parts.next().unwrap_or_default();
    let counter = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
    (format!("{date}-{time}"), counter)
}

/// Conflict copies sitting beside the collection, oldest first.
pub fn conflict_copies(path: &Path) -> Vec<PathBuf> {
    let Some(parent) = path.parent() else {
        return Vec::new();
    };
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let prefix = format!("{stem}.conflict-");
    let mut found: Vec<PathBuf> = match fs::read_dir(parent) {
        Ok(entries) => entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().starts_with(&prefix))
                    .unwrap_or(false)
                    && p.extension().and_then(|e| e.to_str()) == Some("json")
            })
            .collect(),
        Err(_) => return Vec::new(),
    };
    found
        .sort_by_cached_key(|p| backup_order(&p.file_name().unwrap_or_default().to_string_lossy()));
    found
}

/// Put a kept copy back as the live collection, saving what is there now so a
/// mistaken restore is itself recoverable. Returns where the displaced
/// collection went, when there was one.
pub fn restore_from(backup: &Path, path: &Path) -> Result<Option<PathBuf>> {
    // An empty file loads as an empty collection, which would read as a
    // successful restore while wiping every deck. Refuse it by name.
    if fs::metadata(backup).map(|m| m.len() == 0).unwrap_or(false) {
        bail!(
            "{} is empty — there is nothing in it to restore",
            backup.display()
        );
    }
    // Parse it before trusting it: restoring an unreadable file would replace
    // a working collection with one gram cannot open.
    let db = load_from(backup).with_context(|| format!("reading {}", backup.display()))?;
    let displaced = if path.exists() {
        Some(keep_conflicting_copy(path)?)
    } else {
        None
    };
    save_to(&db, path)?;
    Ok(displaced)
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
    Confusing,
    Ignore,
}

fn classify(header: &str) -> Column {
    // Spaces and punctuation go too, so `Part of speech` and `confused with`
    // match their run-together spellings below.
    let h: String = header
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase();
    match h.as_str() {
        "term" | "word" | "front" | "headword" | "vocab" | "vocabulary" => Column::Term,
        "pron" | "pronunciation" | "phonetic" | "ipa" | "say" => Column::Pron,
        "pos" | "part" | "partofspeech" | "speech" | "type" => Column::Pos,
        "definition" | "def" | "meaning" | "back" | "translation" | "gloss" => Column::Definition,
        "example" | "examples" | "sentence" | "sentences" | "usage" | "context" => Column::Example,
        "notes" | "note" | "extra" | "comment" | "related" => Column::Notes,
        "confusing" | "confusable" | "confusables" | "confused" | "confusedwith"
        | "lookalike" | "lookalikes" | "similar" => Column::Confusing,
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
                Column::Confusing,
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
        let mut fields = [""; 7];
        for (i, value) in record.iter().enumerate() {
            let slot = match columns.get(i) {
                Some(Column::Term) => 0,
                Some(Column::Pron) => 1,
                Some(Column::Pos) => 2,
                Some(Column::Definition) => 3,
                Some(Column::Example) => 4,
                Some(Column::Notes) => 5,
                Some(Column::Confusing) => 6,
                _ => continue,
            };
            fields[slot] = value.trim();
        }
        let [term, pron, pos, definition, example, notes, confusing] = fields;
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
            existing.confusing = confusing.to_string();
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
            card.confusing = confusing.to_string();
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
    writer.write_record([
        "term",
        "pron",
        "pos",
        "definition",
        "example",
        "notes",
        "confusing",
    ])?;
    for card in &deck.cards {
        writer.write_record([
            &card.term,
            &card.pron,
            &card.pos,
            &card.definition,
            &card.example,
            &card.notes,
            &card.confusing,
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

    /// Two conflicts inside the same second must not share a name: the second
    /// backup would otherwise destroy the collection the first one saved.
    #[test]
    fn repeated_conflicts_each_get_their_own_name() {
        let dir = std::env::temp_dir().join(format!("gram-conflict2-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("collection.json");

        fs::write(&path, r#"{"decks":[{"id":1,"name":"first","cards":[]}]}"#).unwrap();
        let first = keep_conflicting_copy(&path).unwrap();
        fs::write(&path, r#"{"decks":[{"id":2,"name":"second","cards":[]}]}"#).unwrap();
        let second = keep_conflicting_copy(&path).unwrap();
        fs::write(&path, r#"{"decks":[{"id":3,"name":"third","cards":[]}]}"#).unwrap();
        let third = keep_conflicting_copy(&path).unwrap();

        assert_ne!(first, second);
        assert_ne!(second, third);
        assert_eq!(load_from(&first).unwrap().decks[0].name, "first");
        assert_eq!(load_from(&second).unwrap().decks[0].name, "second");
        assert_eq!(load_from(&third).unwrap().decks[0].name, "third");

        let found = conflict_copies(&path);
        assert_eq!(found.len(), 3, "all three are listed: {found:?}");
        assert_eq!(found[0], first, "and the oldest sorts first");
        assert!(
            !found.contains(&path),
            "the collection itself is not a backup"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_backup_is_labelled_by_its_stamp() {
        let at = |name: &str| backup_label(&PathBuf::from("/data/gram").join(name));
        assert_eq!(
            at("collection.conflict-20261006-120000.json"),
            "20261006-120000"
        );
        assert_eq!(
            at("collection.conflict-20261006-120000-3.json"),
            "20261006-120000-3",
            "the counter is part of which backup it is"
        );
        // Anything not shaped like one keeps its name, so `:restore some.json`
        // still reports something recognisable.
        assert_eq!(at("some.json"), "some.json");
        assert_eq!(at("collection.json"), "collection.json");
        assert_eq!(at("notes.txt"), "notes.txt");
    }

    #[test]
    fn restoring_keeps_what_it_displaces() {
        let dir = std::env::temp_dir().join(format!("gram-restore-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("collection.json");

        let backup = dir.join("kept.json");
        fs::write(&backup, r#"{"decks":[{"id":7,"name":"kept","cards":[]}]}"#).unwrap();
        fs::write(&path, r#"{"decks":[{"id":1,"name":"live","cards":[]}]}"#).unwrap();

        let displaced = restore_from(&backup, &path)
            .unwrap()
            .expect("one was there");
        assert_eq!(load_from(&path).unwrap().decks[0].name, "kept");
        assert_eq!(
            load_from(&displaced).unwrap().decks[0].name,
            "live",
            "a mistaken restore is itself undoable"
        );

        // A backup gram cannot parse is refused before anything is replaced.
        let broken = dir.join("broken.json");
        fs::write(&broken, "{ not json").unwrap();
        assert!(restore_from(&broken, &path).is_err());
        assert_eq!(
            load_from(&path).unwrap().decks[0].name,
            "kept",
            "the collection is left alone"
        );

        // So is an empty one: it loads as an empty collection, so restoring it
        // would read as a success while wiping every deck.
        let empty = dir.join("empty.json");
        fs::write(&empty, "").unwrap();
        assert!(restore_from(&empty, &path).is_err());
        assert_eq!(
            load_from(&path).unwrap().decks[0].name,
            "kept",
            "still there"
        );

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
    fn imports_confusing_words() {
        let (deck, _) = import_str(
            "term,definition,confused with\n\
             affect,to influence,\"effect — a result\neffete\"\n",
            "confusing.csv",
        );
        assert_eq!(deck.cards[0].confusing, "effect — a result\neffete");
        assert_eq!(
            deck.cards[0].confusables().collect::<Vec<_>>(),
            ["effect — a result", "effete"]
        );
    }

    #[test]
    fn an_empty_confusing_field_stays_out_of_the_collection() {
        let card = Card::new(1, "abash".into(), "to embarrass".into(), today());
        let json = serde_json::to_string(&card).unwrap();
        assert!(!json.contains("confusing"), "{json}");
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
