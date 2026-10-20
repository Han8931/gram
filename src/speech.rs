//! Saying a word out loud, through whatever the system speaks with: `say` on
//! macOS, `espeak-ng` or `espeak` elsewhere.
//!
//! The language is the deck's, not the word's. A Japanese word written all in
//! kanji is indistinguishable from Chinese on its own, but a deck with any
//! kana in it is a Japanese deck.

use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    /// Whatever the system voice is — English, for most people.
    Default,
    Chinese,
    Japanese,
    Korean,
}

impl Language {
    /// What the deck's terms are written in.
    pub fn of<'a>(terms: impl IntoIterator<Item = &'a str>) -> Self {
        let (mut han, mut kana, mut hangul) = (false, false, false);
        for ch in terms.into_iter().flat_map(str::chars) {
            match ch as u32 {
                0x3040..=0x30ff | 0x31f0..=0x31ff => kana = true,
                0xac00..=0xd7af | 0x1100..=0x11ff | 0x3130..=0x318f => hangul = true,
                0x4e00..=0x9fff | 0x3400..=0x4dbf | 0xf900..=0xfaff => han = true,
                _ => {}
            }
        }
        match (kana, hangul, han) {
            (true, _, _) => Self::Japanese,
            (_, true, _) => Self::Korean,
            (_, _, true) => Self::Chinese,
            _ => Self::Default,
        }
    }

    /// The locale `say -v '?'` lists a voice under.
    fn locale(self) -> Option<&'static str> {
        match self {
            Self::Default => None,
            Self::Chinese => Some("zh_CN"),
            Self::Japanese => Some("ja_JP"),
            Self::Korean => Some("ko_KR"),
        }
    }

    /// The voice name `espeak` takes for it.
    fn espeak(self) -> Option<&'static str> {
        match self {
            Self::Default => None,
            Self::Chinese => Some("cmn"),
            Self::Japanese => Some("ja"),
            Self::Korean => Some("ko"),
        }
    }

    /// The voices macOS has shipped for years, tried before anything else
    /// the listing offers: the novelty voices sort first and read badly.
    fn preferred(self) -> &'static [&'static str] {
        match self {
            Self::Default => &[],
            Self::Chinese => &["Tingting", "Ting-Ting"],
            Self::Japanese => &["Kyoko", "Otoya"],
            Self::Korean => &["Yuna"],
        }
    }
}

/// The program that does the speaking, found once.
fn program() -> Option<&'static str> {
    static FOUND: OnceLock<Option<&'static str>> = OnceLock::new();
    *FOUND.get_or_init(|| {
        ["say", "espeak-ng", "espeak"]
            .into_iter()
            .find(|name| on_path(name))
    })
}

fn on_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(name).is_file()))
}

/// `say -v '?'`, read once: one voice per line, `Name  locale  # sample`.
fn say_voices() -> &'static [(String, String)] {
    static VOICES: OnceLock<Vec<(String, String)>> = OnceLock::new();
    VOICES.get_or_init(|| {
        let Ok(out) = Command::new("say").args(["-v", "?"]).output() else {
            return Vec::new();
        };
        parse_say_voices(&String::from_utf8_lossy(&out.stdout))
    })
}

fn parse_say_voices(listing: &str) -> Vec<(String, String)> {
    listing
        .lines()
        .filter_map(|line| {
            let before = line.split('#').next()?.trim_end();
            // The name can have spaces in it — `Eddy (Chinese (China
            // mainland))` — so the locale is the last word, not the second.
            let (name, locale) = before.rsplit_once(char::is_whitespace)?;
            Some((name.trim().to_string(), locale.to_string()))
        })
        .collect()
}

/// A voice for `language` from a `say` listing: a familiar one if it is
/// installed, else the first the listing has for that locale.
fn pick_say_voice(voices: &[(String, String)], language: Language) -> Option<String> {
    let locale = language.locale()?;
    let local: Vec<&String> = voices
        .iter()
        .filter(|(_, l)| l == locale)
        .map(|(name, _)| name)
        .collect();
    language
        .preferred()
        .iter()
        .find_map(|want| local.iter().find(|name| name.as_str() == *want))
        .or_else(|| local.first())
        .map(|name| name.to_string())
}

/// The command that says `text`. `voice` is the deck's own choice and wins
/// outright; without one, the language picks.
fn command(program: &str, voice: Option<&str>, language: Language, text: &str) -> Command {
    let voice = match voice {
        Some(voice) => Some(voice.to_string()),
        None if program == "say" => pick_say_voice(say_voices(), language),
        None => language.espeak().map(str::to_string),
    };
    let mut command = Command::new(program);
    if let Some(voice) = voice {
        command.args(["-v", &voice]);
    }
    // `--` so a word starting with a dash is a word, not an option.
    command.arg("--").arg(text);
    command
}

/// What to hand the voice for `term`. Lower case, because decks capitalise
/// their headwords and a capital tells the voice it is reading a name:
/// Samantha says Word Smart's `Abate` like the Italian surname, ah-BAH-tay,
/// and `abate` like the word. Scripts without case are left as they are.
pub fn spoken(term: &str) -> String {
    term.trim().to_lowercase()
}

/// Speaks one word at a time: a new word cuts the last one off rather than
/// queueing behind it.
#[derive(Default)]
pub struct Speaker {
    child: Option<Child>,
}

impl Speaker {
    pub fn say(
        &mut self,
        text: &str,
        voice: Option<&str>,
        language: Language,
    ) -> Result<(), String> {
        let Some(program) = program() else {
            return Err("nothing to speak with — install espeak-ng".to_string());
        };
        self.stop();
        // Its output would land on top of the TUI.
        let child = command(program, voice, language, text)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| format!("{program}: {err}"))?;
        self.child = Some(child);
        Ok(())
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Speaker {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(command: &Command) -> Vec<String> {
        command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn a_deck_is_known_by_what_its_terms_are_written_in() {
        assert_eq!(Language::of(["abash", "abate"]), Language::Default);
        assert_eq!(Language::of(["你好", "谢谢"]), Language::Chinese);
        // All kanji on its own, but the deck has kana elsewhere.
        assert_eq!(Language::of(["学生", "ありがとう"]), Language::Japanese);
        assert_eq!(Language::of(["한국어"]), Language::Korean);
    }

    #[test]
    fn the_say_listing_keeps_names_with_spaces_in_them() {
        let listing = "\
Eddy (Chinese (China mainland)) zh_CN    # 你好！我叫Eddy。
Kyoko               ja_JP    # こんにちは！ 私の名前はKyokoです。
Tingting            zh_CN    # 你好！我叫婷婷。
";
        let voices = parse_say_voices(listing);
        assert_eq!(
            voices[0],
            ("Eddy (Chinese (China mainland))".into(), "zh_CN".into())
        );
        assert_eq!(
            pick_say_voice(&voices, Language::Chinese).as_deref(),
            Some("Tingting"),
            "the familiar voice beats the one listed first"
        );
        assert_eq!(
            pick_say_voice(&voices, Language::Japanese).as_deref(),
            Some("Kyoko")
        );
        assert_eq!(pick_say_voice(&voices, Language::Korean), None);
        assert_eq!(pick_say_voice(&voices, Language::Default), None);
    }

    #[test]
    fn the_first_voice_for_a_locale_will_do() {
        let voices = parse_say_voices("Flo (Korean)  ko_KR  # 안녕하세요\n");
        assert_eq!(
            pick_say_voice(&voices, Language::Korean).as_deref(),
            Some("Flo (Korean)")
        );
    }

    #[test]
    fn headwords_are_spoken_as_words_not_names() {
        assert_eq!(spoken("Abate"), "abate");
        assert_eq!(spoken(" Bona fide "), "bona fide");
        assert_eq!(spoken("你好"), "你好");
    }

    #[test]
    fn the_decks_own_voice_wins() {
        let cmd = command("espeak-ng", Some("en-gb"), Language::Chinese, "hello");
        assert_eq!(args(&cmd), ["-v", "en-gb", "--", "hello"]);
        let cmd = command("espeak-ng", None, Language::Japanese, "ありがとう");
        assert_eq!(args(&cmd), ["-v", "ja", "--", "ありがとう"]);
        let cmd = command("espeak-ng", None, Language::Default, "-dash");
        assert_eq!(args(&cmd), ["--", "-dash"], "a word, not an option");
    }
}
