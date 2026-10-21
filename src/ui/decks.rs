//! The deck list.

use super::*;

pub(super) fn render_decks(frame: &mut Frame, app: &mut App, area: Rect) {
    if app.db.decks.is_empty() {
        let text = Text::from(vec![
            Line::from(""),
            Line::from("  No decks yet.").bold(),
            Line::from(""),
            // Nothing here names a file that ships with the program: the word
            // list is built from your own copy of the book.
            Line::from(Span::styled(
                "  :import <file.csv> [DECK]    load a CSV or TSV",
                Style::default().fg(DIM),
            )),
            Line::from(Span::styled(
                "  :deck <name>                 create an empty deck",
                Style::default().fg(DIM),
            )),
            Line::from(Span::styled(
                "  a                            add a card by hand",
                Style::default().fg(DIM),
            )),
            Line::from(Span::styled(
                "  ?                            keybindings",
                Style::default().fg(DIM),
            )),
        ]);
        frame.render_widget(Paragraph::new(text).block(panel("decks")), area);
        return;
    }

    let today = app.today;

    // Draw the frame first, then a header strip and the rows inside it.
    let block = panel("decks");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [head_area, rows_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);

    frame.render_widget(
        table_header(Line::from(vec![
            Span::styled(format!(" {:<26}", "DECK"), Style::default().fg(DIM).bold()),
            Span::styled(format!("{:>6}", "NEW"), Style::default().fg(NEW).bold()),
            Span::styled(format!("{:>6}", "LEARN"), Style::default().fg(LEARN).bold()),
            Span::styled(format!("{:>6}", "DUE"), Style::default().fg(DUE).bold()),
            Span::styled(format!("{:>7}", "CARDS"), Style::default().fg(DIM).bold()),
            Span::styled(format!("{:>5}", "★"), Style::default().fg(MARK).bold()),
            Span::styled("  STATUS", Style::default().fg(DIM).bold()),
        ])),
        head_area,
    );

    let items: Vec<ListItem> = app
        .db
        .decks
        .iter()
        .map(|deck| {
            let counts = deck.counts(today);
            let status = counts.status();
            let name_style = if counts.pending() == 0 {
                Style::default().fg(DIM)
            } else {
                Style::default().fg(BRIGHT).bold()
            };
            ListItem::new(Line::from(vec![
                Span::styled(pad(&deck.name, 26), name_style),
                count_span(counts.new, NEW),
                count_span(counts.learning, LEARN),
                count_span(counts.due, DUE),
                Span::styled(format!("{:>7}", counts.total), Style::default().fg(DIM)),
                if counts.flagged > 0 {
                    Span::styled(
                        format!("{:>5}", counts.flagged),
                        Style::default().fg(MARK).bold(),
                    )
                } else {
                    Span::styled(format!("{:>5}", "·"), Style::default().fg(DIM))
                },
                // Padded to the widest label, so whatever follows it down the
                // list lines up.
                Span::styled(
                    format!("  {}", pad(status.label(), 14)),
                    Style::default().fg(status_color(status)).bold(),
                ),
                // Once the day's ration is spent the NEW column reads `·`,
                // which would otherwise look like a deck you had finished.
                // Say how much of it is still unlearnt, and that n gets at it.
                if counts.new == 0 && counts.unseen > 0 {
                    Span::styled(
                        format!("{} unseen — n to learn", counts.unseen),
                        Style::default().fg(DIM),
                    )
                } else {
                    Span::raw("")
                },
                if counts.suspended > 0 {
                    Span::styled(
                        format!("  ({} suspended)", counts.suspended),
                        Style::default().fg(DIM),
                    )
                } else {
                    Span::raw("")
                },
                if deck.reverse {
                    Span::styled("  ⇄ reverse", Style::default().fg(ACCENT))
                } else {
                    Span::raw("")
                },
            ]))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(Style::default().bg(SEL_BG).bold())
        .highlight_symbol("▌");

    // Start from the window we drew last time: the widget then scrolls by the
    // least it can to keep the cursor in view, so going back up walks the
    // cursor up the window instead of pinning it to the bottom row.
    let mut state = ListState::default().with_offset(app.decks_off);
    state.select(Some(app.decks_sel));
    frame.render_stateful_widget(list, rows_area, &mut state);
    app.decks_off = state.offset();
}

/// A covered meaning. Dots rather than blanks: a hidden row still has to read
/// as a row with something behind it, not as a card missing its definition.
pub(super) fn mask(columns: usize) -> String {
    "·".repeat(columns.min(12))
}

pub(super) fn count_span(count: usize, color: Color) -> Span<'static> {
    if count == 0 {
        // A dot reads as "nothing here" far faster than a zero does.
        Span::styled(format!("{:>6}", "·"), Style::default().fg(DIM))
    } else {
        Span::styled(format!("{count:>6}"), Style::default().fg(color).bold())
    }
}
