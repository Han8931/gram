use super::*;

#[test]
fn truncate_leaves_short_text_alone() {
    assert_eq!(truncate("abash", 10), "abash");
}

#[test]
fn truncate_adds_an_ellipsis() {
    assert_eq!(truncate("circumlocution", 8), "circuml…");
    assert_eq!(truncate("abc", 1), "…");
}

#[test]
fn wrapped_lines_counts_wrapping() {
    assert_eq!(wrapped_lines("short", 40), 1);
    assert_eq!(wrapped_lines("one two three four five six", 11), 3);
    assert_eq!(wrapped_lines("", 40), 1);
    assert_eq!(wrapped_lines("anything", 0), 1);
}

#[test]
fn window_shows_the_whole_value_when_it_fits() {
    assert_eq!(window("abash", 0, 20), ("abash".to_string(), 0));
}

#[test]
fn window_scrolls_to_keep_the_cursor_visible() {
    let text = "to make ashamed; to embarrass";
    let (visible, offset) = window(text, text.len(), 10);
    assert_eq!(visible, "embarrass");
    assert_eq!(offset, 9, "cursor sits just past the last character");
}

#[test]
fn window_handles_zero_width() {
    assert_eq!(window("abash", 3, 0), (String::new(), 0));
}

#[test]
fn the_wildmenu_shows_every_candidate_that_fits() {
    assert_eq!(wildmenu_window(&[5, 5, 5], 0, 20), 0..3);
}

#[test]
fn the_wildmenu_scrolls_to_keep_the_picked_candidate_in_view() {
    let widths = [5; 10];
    // 12 columns, less two for the markers, holds two candidates.
    assert_eq!(wildmenu_window(&widths, 0, 12), 0..2);
    assert_eq!(wildmenu_window(&widths, 1, 12), 0..2);
    assert_eq!(wildmenu_window(&widths, 2, 12), 1..3);
    assert_eq!(wildmenu_window(&widths, 9, 12), 8..10);
}

#[test]
fn the_wildmenu_still_shows_a_candidate_wider_than_the_strip() {
    assert_eq!(wildmenu_window(&[4, 40, 4], 1, 10), 1..2);
}

#[test]
fn centered_fixed_shrinks_to_fit() {
    let area = Rect {
        x: 0,
        y: 0,
        width: 40,
        height: 10,
    };
    let popup = centered_fixed(80, 20, area);
    assert_eq!((popup.width, popup.height), (40, 10));

    let popup = centered_fixed(20, 4, area);
    assert_eq!(
        (popup.x, popup.y, popup.width, popup.height),
        (10, 3, 20, 4)
    );
}

/// The plain text of a rendered line, for asserting on layout.
fn flat(line: &Line) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

/// A marker-less block, for exercising the wrapper on its own.
fn plain(prefix: &str) -> Prose {
    Prose {
        marker: Span::raw(prefix.to_string()),
        continuation: Span::raw(" ".repeat(prefix.chars().count())),
        body: Style::default().fg(TEXT),
        emphasis: Style::default().fg(MARK).italic(),
    }
}

#[test]
fn wrapped_examples_keep_a_hanging_indent() {
    let text = "While most people would quickly recover from a stumble on \
                stage, Mia felt abject humiliation.";
    let lines = wrap_prose(&Prose::example(), text, 40);
    assert!(lines.len() > 1, "this should need wrapping");
    assert!(flat(&lines[0]).starts_with("  • While"));
    for line in &lines[1..] {
        assert!(
            flat(line).starts_with("    ") && !flat(line).starts_with("    •"),
            "continuation lines align under the bullet text: {:?}",
            flat(line)
        );
    }
    for line in &lines {
        assert!(flat(line).chars().count() <= 40, "{:?}", flat(line));
    }
}

#[test]
fn confusing_words_close_the_card() {
    let mut card = Card::new(
        1,
        "affect".into(),
        "to influence".into(),
        chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
    );
    card.notes = "a note".into();
    card.confusing = "effect — a result\n\neffete".into();
    let text: Vec<String> = card_body(&card, 60, false).iter().map(flat).collect();
    let heading = text.iter().position(|l| l.contains("don't confuse with"));
    let note = text.iter().position(|l| l.contains("a note"));
    assert!(heading > note, "after the notes: {text:?}");
    assert_eq!(
        &text[heading.unwrap() + 1..],
        ["  ≠ effect — a result", "  ≠ effete"]
    );

    card.confusing.clear();
    let text: Vec<String> = card_body(&card, 60, false).iter().map(flat).collect();
    assert!(!text.iter().any(|l| l.contains("confuse")), "{text:?}");
}

#[test]
fn notes_carry_a_gutter_on_every_line() {
    let text = "To do something without shame or embarrassment is to do it \
                unabashedly, which is a longer note than one line holds.";
    let lines = wrap_prose(&Prose::note(), text, 40);
    assert!(lines.len() > 1, "this should need wrapping");
    for line in &lines {
        assert!(
            flat(line).starts_with("  │ "),
            "a note stays behind its rule: {:?}",
            flat(line)
        );
    }
}

#[test]
fn a_note_reads_dimmer_than_an_example() {
    let body = |prose| {
        wrap_prose(&prose, "plain words", 40).remove(0).spans[1]
            .style
            .fg
    };
    assert_eq!(body(Prose::example()), Some(TEXT));
    assert_eq!(body(Prose::note()), Some(DIM));
}

#[test]
fn a_field_wraps_at_spaces_and_at_escaped_newlines() {
    let lines = wrap_field("to step down from power", 12);
    let text: Vec<&str> = lines.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(text, vec!["to step down", "from power"]);
    for (line, _) in &lines {
        assert!(line.chars().count() <= 12, "{line:?}");
    }

    let lines = wrap_field("one\\ntwo", 40);
    let text: Vec<&str> = lines.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(
        text,
        vec!["one\\n", "two"],
        "the editor's \\n breaks the line and stays on the end of it"
    );

    let lines = wrap_field("antidisestablishmentarianism", 10);
    assert_eq!(lines.len(), 3, "a long word is broken rather than hidden");
    let rebuilt: String = lines.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(rebuilt, "antidisestablishmentarianism");

    // A card holding a literal backslash and an n: `\\n` is that backslash,
    // and the n after it is a letter, not half of a line break.
    let lines = wrap_field("one\\\\ntwo", 40);
    let text: Vec<&str> = lines.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(text, vec!["one\\\\ntwo"], "one line, not two");
}

#[test]
fn every_character_of_a_field_has_a_cell_of_its_own() {
    // The cursor counts the value's characters while the field is laid out
    // for display, so the two have to agree: any value character the layout
    // draws nothing for is a cell two cursor positions share, which reads as
    // the cursor refusing to move. Typing a space used to do exactly that.
    for (value, width) in [
        ("to make ashamed", 40),
        ("a definition that is long enough to wrap twice over", 20),
        ("trailing space ", 40),
        ("two  spaces", 40),
        (" leading space", 40),
        ("one\\ntwo\\nthree", 40),
        ("ends with a break\\n", 40),
        ("\\nopens with one", 40),
        ("한국어 공부합니다 and some english", 12),
        ("antidisestablishmentarianism", 10),
        ("", 40),
    ] {
        let lines = wrap_field(value, width);
        let mut seen: Vec<((usize, usize), usize)> = Vec::new();
        for cursor in 0..=util::cluster_count(value) {
            let at = cursor_at(&lines, cursor);
            if let Some((_, other)) = seen.iter().find(|(cell, _)| *cell == at) {
                panic!(
                    "{value:?} at width {width}: characters {other} and {cursor} \
                     both sit at row {} column {} — {lines:?}",
                    at.0, at.1
                );
            }
            seen.push((at, cursor));
        }
    }
}

#[test]
fn wide_characters_wrap_by_column_not_by_count() {
    // Eight Korean syllables are sixteen columns, so twelve columns hold
    // six of them — counting characters would have fitted twelve and
    // overflowed the field by half its width.
    let lines = wrap_field("한국어공부합니다", 12);
    assert_eq!(lines.len(), 2);
    for (text, _) in &lines {
        assert!(util::width(text) <= 12, "{text:?} is too wide");
    }
    let rebuilt: String = lines.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(rebuilt, "한국어공부합니다");

    // The cursor's column is its width, not its index: three syllables in
    // is six columns across.
    assert_eq!(cursor_at(&lines, 3), (0, 6));
}

#[test]
fn the_command_line_window_scrolls_by_column() {
    // Ten columns of a Korean line: five syllables, with the last column
    // kept for the cursor.
    let (visible, column) = window("한국어공부합니다", 8, 10);
    assert!(util::width(&visible) <= 10, "{visible:?}");
    assert_eq!(visible, "부합니다");
    assert_eq!(column, 8, "four syllables behind the cursor");

    // A combining accent takes no column of its own.
    let (visible, column) = window("e\u{301}x", 2, 10);
    assert_eq!(visible, "e\u{301}x");
    assert_eq!(column, 2);
}

#[test]
fn the_cursor_follows_the_text_onto_the_next_line() {
    let value = "to step down from power";
    let lines = wrap_field(value, 12);

    assert_eq!(cursor_at(&lines, 0), (0, 0), "start of the first line");
    assert_eq!(cursor_at(&lines, 7), (0, 7), "still on it");
    // "from power" starts at character 13, just past the dropped space.
    assert_eq!(cursor_at(&lines, 13), (1, 0));
    assert_eq!(cursor_at(&lines, 17), (1, 4));
    assert_eq!(
        cursor_at(&lines, value.chars().count()),
        (1, 10),
        "past the last character, where insert mode leaves it"
    );

    // Across an explicit break, every character of the value has a cell of
    // its own — including the two the marker is written with, which is what
    // keeps the cursor moving on every press.
    let value = "one\\ntwo";
    let lines = wrap_field(value, 40);
    assert_eq!(cursor_at(&lines, 3), (0, 3), "on the backslash");
    assert_eq!(cursor_at(&lines, 4), (0, 4), "on its n");
    assert_eq!(cursor_at(&lines, 5), (1, 0), "the line the break opened");

    let cells: Vec<(usize, usize)> = (0..util::cluster_count(value))
        .map(|cursor| cursor_at(&lines, cursor))
        .collect();
    let mut sorted = cells.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        cells.len(),
        "no two positions share a cell: {cells:?}"
    );
}

#[test]
fn fields_are_sized_to_what_they_hold() {
    // Room to spare: every field gets what it asks for, prose capped.
    let heights = field_heights(40, [1, 1, 1, 2, 6, 20, 5]);
    assert_eq!(
        heights,
        [1, 1, 1, 2, 6, 8, 3],
        "Notes capped at eight rows, Confusing at three"
    );

    // An empty card still gets a row per field to type into.
    assert_eq!(field_heights(40, [0; 7]), [1; 7]);

    // Tight: the greediest field gives rows up until the form fits.
    let heights = field_heights(14, [1, 1, 1, 3, 8, 8, 3]);
    assert_eq!(
        heights.iter().sum::<u16>() + 6,
        14,
        "fields plus the gaps between them fill the height exactly"
    );
    assert!(
        heights.iter().all(|h| *h >= 1),
        "no field is squeezed out: {heights:?}"
    );

    // Nothing left to give: every field keeps its single row and the
    // dialog is simply taller than the terminal, which clamps it.
    assert_eq!(field_heights(1, [4; 7]), [1; 7]);
}

#[test]
fn the_preview_sits_under_the_list_when_there_are_rows_to_spare() {
    for width in [80, 120] {
        let (list, preview) = preview_split(Rect::new(0, 1, width, 30));
        let preview = preview.expect("30 rows is room enough");
        assert_eq!(
            preview.width, width,
            "full width, however wide the terminal"
        );
        assert_eq!(preview.height, PREVIEW_HEIGHT);
        assert_eq!(list.height, 30 - PREVIEW_HEIGHT);
        assert_eq!(preview.y, list.y + list.height, "directly below it");
    }

    let small = Rect::new(0, 1, 120, 18);
    let (list, preview) = preview_split(small);
    assert!(preview.is_none(), "no rows to spare, no preview");
    assert_eq!(list, small, "the list keeps everything");
}

#[test]
fn centering_caps_the_measure_but_never_narrows_a_small_terminal() {
    let wide = centered(Rect::new(0, 2, 120, 10), 82);
    assert_eq!((wide.x, wide.width), (19, 82), "centred within 120 columns");
    assert_eq!(wide.y, 2, "vertical placement is left alone");

    let narrow = Rect::new(0, 0, 60, 10);
    assert_eq!(centered(narrow, 82), narrow, "keeps every column it has");
}

#[test]
fn wrapping_preserves_every_word() {
    let text = "the agony *abated* today and never came back at all";
    let joined = wrap_prose(&plain("  "), text, 20)
        .iter()
        .map(|l| flat(l).trim().to_string())
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(joined, "the agony abated today and never came back at all");
}

#[test]
fn wrapping_highlights_only_the_marked_word() {
    let lines = wrap_prose(&plain("  "), "the agony *abated* today", 60);
    let marked: Vec<&str> = lines[0]
        .spans
        .iter()
        .filter(|s| s.style.fg == Some(MARK))
        .map(|s| s.content.as_ref())
        .collect();
    assert_eq!(marked, vec!["abated"]);
}

#[test]
fn emphasis_inside_a_word_does_not_split_it() {
    let words = emphasised_words("he did it un*abash*edly today");
    let shapes: Vec<usize> = words.iter().map(|w| w.len()).collect();
    assert_eq!(shapes, vec![1, 1, 1, 3, 1], "unabashedly stays one word");
    let rebuilt: String = words[3].iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(rebuilt, "unabashedly");
}

#[test]
fn wrapping_handles_a_single_overlong_word() {
    let lines = wrap_prose(&plain("  "), "antidisestablishmentarianism", 10);
    assert_eq!(lines.len(), 1, "nothing to break on, so it overflows");
    assert_eq!(flat(&lines[0]), "  antidisestablishmentarianism");
}

#[test]
fn truncate_counts_characters_not_bytes() {
    assert_eq!(truncate("naïveté", 7), "naïveté");
    assert_eq!(truncate("naïveté", 4), "naï…");
}
