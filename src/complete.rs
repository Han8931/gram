//! Tab completion for the `:` command line.
//!
//! One pure entry point, [`candidates`], so the behaviour is testable without
//! a terminal: given the text typed so far it says where the word being
//! completed starts and what could finish it.

use std::fs;
use std::path::PathBuf;

use crate::store;

/// Commands Tab offers, sorted so the list never reorders itself under you.
/// Aliases are left out unless they are shorter than what they alias — `:q`
/// earns its place, `:nohlsearch` does not.
pub const COMMANDS: &[&str] = &[
    "add", "b", "browse", "deck", "decks", "export", "forget", "h", "help", "import", "marked",
    "noh", "q", "q!", "qa", "qa!", "quit", "rename", "reset", "restore", "s", "set", "stats",
    "status", "study", "w", "wq", "write", "x",
];

/// Values for `:status` and `:marked`.
const STATUSES: &[&str] = &["marked", "none", "review", "test"];

/// Scopes for `:study`.
const SCOPES: &[&str] = &["due", "flagged", "marked", "more", "new", "review", "test"];

/// Settings for `:set`, written as the `key=` they are typed with.
const SETTINGS: &[&str] = &["grading=", "new=", "reverse=", "voice="];

/// Values for `:set grading=`.
const GRADINGS: &[&str] = &["binary", "sm2"];

/// Values for `:set reverse=`.
const SWITCHES: &[&str] = &["off", "on"];

/// What Tab offers for `line`: the character index where the word being
/// completed starts, and everything that could finish it.
pub fn candidates(line: &str) -> (usize, Vec<String>) {
    let chars: Vec<char> = line.chars().collect();
    // The word under completion runs from after the last space to the end.
    let at = chars
        .iter()
        .rposition(|ch| ch.is_whitespace())
        .map_or(0, |index| index + 1);
    let token: String = chars[at..].iter().collect();

    // The first word is a command name.
    if at == 0 {
        return (0, prefixed(COMMANDS, &token));
    }

    let command = line.split_whitespace().next().unwrap_or_default();
    let found = match command {
        "set" => settings(&token),
        "status" => prefixed(STATUSES, &token),
        "marked" | "mark" | "flagged" => prefixed(STATUSES, &token),
        "study" | "s" => prefixed(SCOPES, &token),
        // `:restore` also takes a plain number, which nothing can complete.
        "import" | "export" | "restore" => paths(&token),
        // `:deck`, `:rename` and the rest take free text; nothing to offer.
        _ => Vec::new(),
    };
    (at, found)
}

/// The longest prefix every candidate shares, which is how far a single Tab
/// can fill in when the choice is still open.
pub fn common_prefix(items: &[String]) -> String {
    let mut iter = items.iter();
    let Some(first) = iter.next() else {
        return String::new();
    };
    let mut common: Vec<char> = first.chars().collect();
    for item in iter {
        let shared = item
            .chars()
            .zip(common.iter())
            .take_while(|(a, b)| a == *b)
            .count();
        common.truncate(shared);
    }
    common.into_iter().collect()
}

fn prefixed(items: &[&str], token: &str) -> Vec<String> {
    items
        .iter()
        .filter(|item| item.starts_with(token))
        .map(|item| item.to_string())
        .collect()
}

/// `:set` takes `key=value`, so completion works on either side of the `=`.
fn settings(token: &str) -> Vec<String> {
    match token.split_once('=') {
        Some(("grading" | "mode", value)) => prefixed(GRADINGS, value)
            .into_iter()
            .map(|grading| format!("grading={grading}"))
            .collect(),
        Some(("reverse", value)) => prefixed(SWITCHES, value)
            .into_iter()
            .map(|switch| format!("reverse={switch}"))
            .collect(),
        // `new=` takes a number, and `:set foo=` is not a setting at all.
        Some(_) => Vec::new(),
        None => prefixed(SETTINGS, token),
    }
}

/// Paths matching `token`, with directories marked by a trailing `/` so a
/// second Tab can carry on into them. Hidden files stay hidden until asked
/// for by name.
fn paths(token: &str) -> Vec<String> {
    let (typed_dir, name) = match token.rfind('/') {
        Some(at) => token.split_at(at + 1),
        None => ("", token),
    };
    let dir = if typed_dir.is_empty() {
        PathBuf::from(".")
    } else {
        match store::expand_tilde(typed_dir) {
            Ok(path) => path,
            Err(_) => return Vec::new(),
        }
    };

    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut found: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let file = entry.file_name().into_string().ok()?;
            if !file.starts_with(name) || (file.starts_with('.') && !name.starts_with('.')) {
                return None;
            }
            let directory = entry.file_type().is_ok_and(|kind| kind.is_dir());
            Some(format!(
                "{typed_dir}{file}{}",
                if directory { "/" } else { "" }
            ))
        })
        .collect();
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(line: &str) -> Vec<String> {
        candidates(line).1
    }

    #[test]
    fn the_first_word_completes_to_commands() {
        let (at, found) = candidates("st");
        assert_eq!(at, 0);
        assert_eq!(found, vec!["stats", "status", "study"]);

        assert_eq!(names("expo"), vec!["export"]);
        assert!(names("zzz").is_empty(), "no such command");
    }

    #[test]
    fn an_empty_line_offers_everything() {
        assert_eq!(names("").len(), COMMANDS.len());
    }

    #[test]
    fn arguments_complete_per_command() {
        let (at, found) = candidates("status re");
        assert_eq!(at, 7, "the word starts after the space");
        assert_eq!(found, vec!["review"]);

        assert_eq!(names("marked t"), vec!["test"]);
        assert_eq!(names("study d"), vec!["due"]);
        assert_eq!(
            names("status "),
            vec!["marked", "none", "review", "test"],
            "a trailing space offers the lot"
        );
        assert!(names("rename anything").is_empty(), "free text");
    }

    #[test]
    fn set_completes_both_sides_of_the_equals() {
        assert_eq!(names("set g"), vec!["grading="]);
        assert_eq!(names("set grading=s"), vec!["grading=sm2"]);
        assert_eq!(
            names("set grading="),
            vec!["grading=binary", "grading=sm2"],
            "both modes"
        );
        assert!(names("set new=2").is_empty(), "a number, not a choice");
        assert_eq!(names("set r"), vec!["reverse="]);
        assert_eq!(names("set reverse=o"), vec!["reverse=off", "reverse=on"]);
    }

    #[test]
    fn import_completes_paths() {
        let dir = std::env::temp_dir().join(format!("gram-complete-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("nested")).unwrap();
        fs::write(dir.join("words.csv"), "term,definition\n").unwrap();
        fs::write(dir.join("worse.tsv"), "term\tdefinition\n").unwrap();
        fs::write(dir.join(".hidden.csv"), "").unwrap();

        let prefix = format!("{}/", dir.display());
        let found = names(&format!("import {prefix}wor"));
        assert_eq!(
            found,
            vec![format!("{prefix}words.csv"), format!("{prefix}worse.tsv")],
            "both matches, path kept as typed"
        );

        let all = names(&format!("import {prefix}"));
        assert!(
            all.contains(&format!("{prefix}nested/")),
            "directories get a trailing slash: {all:?}"
        );
        assert!(
            !all.iter().any(|path| path.contains(".hidden")),
            "hidden files stay hidden: {all:?}"
        );

        let hidden = names(&format!("import {prefix}.h"));
        assert_eq!(hidden, vec![format!("{prefix}.hidden.csv")], "unless asked");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_directory_offers_nothing() {
        assert!(names("import /no/such/place/file").is_empty());
    }

    #[test]
    fn common_prefix_is_as_far_as_one_tab_can_fill() {
        let items: Vec<String> = ["stats", "status", "study"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(common_prefix(&items), "st");

        let one: Vec<String> = vec!["export".to_string()];
        assert_eq!(common_prefix(&one), "export");
        assert_eq!(common_prefix(&[]), "");
    }
}
