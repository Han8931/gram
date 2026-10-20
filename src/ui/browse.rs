//! The card browser: one row per word, and the preview pane below it.

use super::*;

/// Rows the browser needs before it shows a preview under the list, and the
/// rows the preview takes. Under the list rather than beside it: the pane gets
/// the full width for prose, and the list keeps its columns.
pub(super) const PREVIEW_BELOW: u16 = 22;
pub(super) const PREVIEW_HEIGHT: u16 = 10;

/// Where the browser's preview pane goes: under the list when there are rows
/// to spare, nowhere when there are not.
pub(super) fn preview_split(area: Rect) -> (Rect, Option<Rect>) {
    if area.height >= PREVIEW_BELOW {
        let [list, preview] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(PREVIEW_HEIGHT)]).areas(area);
        (list, Some(preview))
    } else {
        (area, None)
    }
}

pub(super) fn render_browse(frame: &mut Frame, app: &mut App, area: Rect, found: &[usize]) {
    let Some(deck) = app.db.deck_by_id(app.browse_deck) else {
        frame.render_widget(Paragraph::new("no deck").block(panel("browse")), area);
        return;
    };

    // Reading a word's examples should not mean opening the editor, so the
    // selected card is shown in full alongside the list where there is room.
    let (area, preview_area) = preview_split(area);

    let title = match (app.filter.is_empty(), app.status_filter) {
        (true, StatusFilter::Off) => format!("{} — {} cards", deck.name, deck.cards.len()),
        (true, filter) => format!("{} — {} {}", deck.name, app.rows.len(), filter.label()),
        (false, StatusFilter::Off) => format!(
            "{} — {}/{} matching {:?}",
            deck.name,
            app.rows.len(),
            deck.cards.len(),
            app.filter
        ),
        (false, filter) => format!(
            "{} — {} {} matching {:?}",
            deck.name,
            app.rows.len(),
            filter.label(),
            app.filter
        ),
    };

    // Say so in the border: a covered list looks like a deck with no
    // definitions in it, and you would go looking for the bug.
    let title = if app.hide_meanings {
        format!("{title} — hidden")
    } else {
        title
    };

    let block = panel(&title);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [head_area, rows_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);

    // 1 for the list cursor, then pos (5), status (10), state (6), due (5),
    // a leading space before the definition, and 2 to keep the last column
    // off the border.
    const FIXED: usize = 30;
    const STATUS_WIDTH: usize = 9;
    let width = inner.width as usize;
    let term_width = 22.min(width / 4).max(6);
    // Wide enough for the last row's number, so the column never shifts as you
    // scroll; two digits minimum, or a one-card deck looks ragged.
    let number_width = app.rows.len().to_string().len().max(2);
    let definition_width = width.saturating_sub(term_width + FIXED + number_width + 1);
    // Pronunciation gets a column when the deck has any and the definition
    // can spare it — for a Chinese or Japanese deck the reading is half the
    // word, and the card pane is not always tall enough to show it.
    const PRON_WIDTH: usize = 16;
    const MIN_DEFINITION: usize = 20;
    let show_pron = deck.cards.iter().any(|card| !card.pron.is_empty())
        && definition_width >= PRON_WIDTH + 1 + MIN_DEFINITION;
    let definition_width = if show_pron {
        definition_width - PRON_WIDTH - 1
    } else {
        definition_width
    };
    let today = app.today;
    // What `/` found, picked out in the list it left whole.
    let found: std::collections::HashSet<usize> = found.iter().copied().collect();

    let items: Vec<ListItem> = app
        .rows
        .iter()
        .filter_map(|i| deck.cards.get(*i))
        .enumerate()
        .map(|(row, card)| {
            let (state_text, state_color) = match card.state {
                _ if card.suspended => ("susp", DIM),
                CardState::New => ("new", NEW),
                CardState::Learning => ("learn", LEARN),
                CardState::Review => ("rev", DUE),
            };

            // Colour the due column by urgency: overdue shouts, far-off whispers.
            let (due, due_color) = if card.suspended || card.state == CardState::New {
                ("—".to_string(), DIM)
            } else {
                match (card.due - today).num_days() {
                    d if d <= 0 => ("due".to_string(), WARN),
                    d if d <= 3 => (format!("{d}d"), MARK),
                    d => (format!("{d}d"), DIM),
                }
            };

            let term_style = if card.suspended {
                Style::default().fg(DIM).add_modifier(Modifier::CROSSED_OUT)
            } else if found.contains(&row) {
                Style::default().fg(MARK).bold().underlined()
            } else {
                Style::default().fg(BRIGHT).bold()
            };

            // The word's own status, as a column you can actually see.
            let status_text = match card.status {
                Study::None => "—".to_string(),
                other => format!("{} {}", other.glyph(), other.label()),
            };

            let mut spans = vec![
                // Where the word sits in the list you are looking at — "word
                // 412 of 832" is how you keep your place in a deck.
                Span::styled(
                    format!("{:>number_width$} ", row + 1),
                    Style::default().fg(DIM),
                ),
                Span::styled(pad(&card.term, term_width), term_style),
            ];
            if show_pron {
                spans.push(Span::styled(
                    format!(" {}", pad(&card.pron, PRON_WIDTH)),
                    Style::default().fg(DIM),
                ));
            }
            spans.extend([
                Span::styled(
                    format!(" {}", pad(&card.pos, 4)),
                    Style::default().fg(DIM).italic(),
                ),
                if app.meaning_hidden(row) {
                    Span::styled(
                        format!(" {}", pad(&mask(definition_width), definition_width)),
                        Style::default().fg(DIM),
                    )
                } else {
                    Span::styled(
                        format!(" {}", pad(&card.definition, definition_width)),
                        Style::default().fg(TEXT),
                    )
                },
                Span::styled(
                    format!(" {}", pad(&status_text, STATUS_WIDTH)),
                    Style::default().fg(study_color(card.status)).bold(),
                ),
                Span::styled(
                    format!(" {state_text:<5}"),
                    Style::default().fg(state_color),
                ),
                Span::styled(format!(" {due:>4}"), Style::default().fg(due_color).bold()),
            ]);
            ListItem::new(Line::from(spans))
        })
        .collect();

    let mut header = vec![
        Span::styled(
            format!(" {:>number_width$} ", "#"),
            Style::default().fg(DIM).bold(),
        ),
        Span::styled(
            format!("{:<width$}", "TERM", width = term_width),
            Style::default().fg(DIM).bold(),
        ),
    ];
    if show_pron {
        header.push(Span::styled(
            format!(" {:<PRON_WIDTH$}", "PRON"),
            Style::default().fg(DIM).bold(),
        ));
    }
    header.extend([
        Span::styled(format!(" {:<4}", "POS"), Style::default().fg(DIM).bold()),
        Span::styled(
            format!(" {:<width$}", "DEFINITION", width = definition_width),
            Style::default().fg(DIM).bold(),
        ),
        Span::styled(
            format!(" {:<STATUS_WIDTH$}", "STATUS"),
            Style::default().fg(MARK).bold(),
        ),
        Span::styled(format!(" {:<5}", "STATE"), Style::default().fg(DIM).bold()),
        Span::styled(format!(" {:>4}", "DUE"), Style::default().fg(DIM).bold()),
    ]);
    frame.render_widget(table_header(Line::from(header)), head_area);

    let list = List::new(items)
        .highlight_style(Style::default().bg(SEL_BG))
        .highlight_symbol("▌");

    // Carried over from the last frame, so the window only moves when the
    // cursor would otherwise leave it. See `render_decks`.
    let mut state = ListState::default().with_offset(app.browse_off);
    if !app.rows.is_empty() {
        state.select(Some(app.browse_sel));
    }
    frame.render_stateful_widget(list, rows_area, &mut state);
    app.browse_off = state.offset();

    if let Some(preview_area) = preview_area {
        render_preview(frame, app, preview_area);
    }
}

/// The selected word in full, beside or below the list.
pub(super) fn render_preview(frame: &mut Frame, app: &App, area: Rect) {
    let mut block = panel("card").padding(Padding::horizontal(1));
    let width = area.width.saturating_sub(4) as usize;

    let card = app
        .selected_card()
        .and_then(|(deck_id, index)| app.db.deck_by_id(deck_id).and_then(|d| d.cards.get(index)));

    // The pane is the answer sheet, so `z` has to cover it too — otherwise
    // the definition is masked in the list and spelled out an inch below it.
    let hidden = app.meaning_hidden(app.browse_sel);

    let lines = match card {
        Some(card) => {
            let mut lines = vec![card_head(card), Line::from("")];
            if hidden {
                lines.extend(card_body_hidden(width));
            } else {
                lines.extend(card_body(card, width, false));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                scheduling_summary(card),
                Style::default().fg(DIM),
            )));
            lines
        }
        None => vec![Line::from(Span::styled(
            "nothing selected",
            Style::default().fg(DIM).italic(),
        ))],
    };

    // A glance, not a reader: say when there is more, and how to read it.
    if lines.len() > area.height.saturating_sub(2) as usize {
        block = block.title_bottom(
            Line::from(vec![
                Span::styled(" … ", Style::default().fg(DIM)),
                Span::styled("Enter", Style::default().fg(ACCENT).bold()),
                Span::styled(" to read it all ", Style::default().fg(DIM)),
            ])
            .right_aligned(),
        );
    }

    // Every line is wrapped already, so there is nothing for the paragraph
    // to wrap — and the count above is the count of rows.
    frame.render_widget(Paragraph::new(lines).block(block), area);
}
