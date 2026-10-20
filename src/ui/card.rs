//! A card, drawn: the head, the body under it, and the prose wrapping both use.

use super::*;

/// The word itself: term, status, pronunciation, part of speech.
pub(super) fn card_head(card: &Card) -> Line<'static> {
    let mut head = vec![Span::styled(
        card.term.clone(),
        Style::default().fg(ACCENT).bold(),
    )];
    if card.status.is_set() {
        head.push(Span::raw("  "));
        head.push(study_span(card.status));
    }
    if !card.pron.is_empty() {
        head.push(Span::styled(
            format!("  ({})", card.pron),
            Style::default().fg(DIM),
        ));
    }
    if !card.pos.is_empty() {
        head.push(Span::styled(
            format!("  {}", card.pos),
            Style::default().fg(DIM).italic(),
        ));
    }
    Line::from(head)
}

/// The same pane with `z` on. The examples go too, not just the definition:
/// a sentence using the word gives away as much as the gloss does.
pub(super) fn card_body_hidden(width: usize) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(mask(width), Style::default().fg(DIM))),
        Line::from(""),
        Line::from(Span::styled(
            "space to show this one, z to show them all",
            Style::default().fg(DIM).italic(),
        )),
    ]
}

/// A small caps label over one part of the card, so a long answer can be
/// read by skimming down its headings.
pub(super) fn section(label: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!("  {label}"),
        Style::default().fg(DIM).bold(),
    ))
}

/// A rule across the card, between the word and what it means.
pub(super) fn rule(width: usize) -> Line<'static> {
    Line::from(Span::styled("─".repeat(width), Style::default().fg(FAINT)))
}

/// Everything behind the word: definition, examples, notes. Shared by the
/// review card, the word view and the browser's preview, so they read the
/// same. `labelled` heads each part with what it is, which the full-size
/// cards have room for and the preview pane does not.
pub(super) fn card_body(card: &Card, width: usize, labelled: bool) -> Vec<Line<'static>> {
    let mut lines = wrap_prose(&Prose::definition(), &card.definition, width);

    // One blank line ahead of each example, so a run of them reads as
    // separate sentences rather than a single grey block.
    let example = Prose::example();
    let mut examples = card.examples().peekable();
    if labelled && examples.peek().is_some() {
        lines.push(Line::from(""));
        lines.push(section("EXAMPLES"));
    }
    for (i, text) in examples.enumerate() {
        if !labelled || i > 0 {
            lines.push(Line::from(""));
        }
        lines.extend(wrap_prose(&example, text, width));
    }

    if !card.notes.is_empty() {
        let note = Prose::note();
        lines.push(Line::from(""));
        if labelled {
            lines.push(section("NOTES"));
        }
        for text in card.notes.lines().filter(|l| !l.trim().is_empty()) {
            lines.extend(wrap_prose(&note, text, width));
        }
    }

    // Look-alikes last, under their own heading, so the word you just learnt
    // is set apart from the ones it is so often mistaken for.
    let mut confusables = card.confusables().peekable();
    if confusables.peek().is_some() {
        let confusable = Prose::confusable();
        lines.push(Line::from(""));
        lines.push(if labelled {
            section("DON'T CONFUSE WITH")
        } else {
            Line::from(Span::styled(
                "  don't confuse with",
                Style::default().fg(WARN).italic(),
            ))
        });
        for text in confusables {
            lines.extend(wrap_prose(&confusable, text, width));
        }
    }

    lines
}

pub(super) fn scheduling_summary(card: &Card) -> String {
    format!(
        "  {} · ease {:.2} · reps {} · lapses {}",
        card.state.label(),
        card.ease,
        card.reps,
        card.lapses
    )
}

/// Split text into whitespace-separated words, each carrying the pieces it is
/// made of and whether each piece sat inside `*emphasis*` markers. A word is
/// kept whole even when a marker falls inside it, as in `un*abash*edly`.
pub(super) fn emphasised_words(text: &str) -> Vec<Vec<(String, bool)>> {
    let mut words: Vec<Vec<(String, bool)>> = Vec::new();
    let mut word: Vec<(String, bool)> = Vec::new();
    let mut piece = String::new();

    for (run, emphasised) in split_emphasis(text) {
        for ch in run.chars() {
            if ch.is_whitespace() {
                if !piece.is_empty() {
                    word.push((std::mem::take(&mut piece), emphasised));
                }
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            } else {
                piece.push(ch);
            }
        }
        if !piece.is_empty() {
            word.push((std::mem::take(&mut piece), emphasised));
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

/// How one wrapped block of prose is painted: the marker on its first line,
/// what continuation lines carry in its place, and the two text styles.
pub(super) struct Prose {
    pub(super) marker: Span<'static>,
    pub(super) continuation: Span<'static>,
    pub(super) body: Style,
    pub(super) emphasis: Style,
}

impl Prose {
    /// The definition: the answer, so the brightest thing on the card, set
    /// in the same column as the bullets' text below it.
    pub(super) fn definition() -> Self {
        let style = Style::default().fg(BRIGHT).bold();
        Self {
            marker: Span::raw("  "),
            continuation: Span::raw("  "),
            body: style,
            emphasis: style.fg(MARK),
        }
    }

    /// An example sentence: an accented bullet, body text, and the target word
    /// picked out so the eye lands on it first.
    pub(super) fn example() -> Self {
        Self {
            marker: Span::styled("  • ", Style::default().fg(ACCENT)),
            continuation: Span::raw("    "),
            body: Style::default().fg(TEXT),
            emphasis: Style::default().fg(MARK).bold().italic(),
        }
    }

    /// A follow-on note. Dimmed, and behind a gutter rule on every line, so it
    /// reads as an aside instead of one more example.
    pub(super) fn note() -> Self {
        let gutter = Style::default().fg(DIM);
        Self {
            marker: Span::styled("  │ ", gutter),
            continuation: Span::styled("  │ ", gutter),
            body: gutter,
            emphasis: Style::default().fg(MARK).italic(),
        }
    }

    /// A word easily mistaken for this one, behind a "not equal" mark.
    pub(super) fn confusable() -> Self {
        Self {
            marker: Span::styled("  ≠ ", Style::default().fg(WARN)),
            continuation: Span::raw("    "),
            body: Style::default().fg(TEXT),
            emphasis: Style::default().fg(MARK).bold().italic(),
        }
    }
}

/// Word-wrap `text` into `width` columns behind the block's marker, giving
/// continuation lines a hanging indent so a bullet's text stays in one column.
/// Words the importer wrapped in `*` are highlighted.
pub(super) fn wrap_prose(prose: &Prose, text: &str, width: usize) -> Vec<Line<'static>> {
    let indent_width = util::width(&prose.marker.content);
    let available = width.saturating_sub(indent_width).max(8);

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut spans: Vec<Span<'static>> = vec![prose.marker.clone()];
    let mut column = 0usize;

    for word in emphasised_words(text) {
        let word_width: usize = word.iter().map(|(piece, _)| util::width(piece)).sum();
        if column > 0 && column + 1 + word_width > available {
            lines.push(Line::from(std::mem::take(&mut spans)));
            spans.push(prose.continuation.clone());
            column = 0;
        }
        if column > 0 {
            spans.push(Span::raw(" "));
            column += 1;
        }
        for (piece, emphasised) in word {
            column += util::width(&piece);
            let style = if emphasised {
                prose.emphasis
            } else {
                prose.body
            };
            spans.push(Span::styled(piece, style));
        }
    }

    lines.push(Line::from(spans));
    lines
}
