//! The review screen and the word view, which share one scrolling card.

use super::*;

pub(super) fn render_review(frame: &mut Frame, app: &mut App, area: Rect) {
    // Taken by value so the card's scroll offset can be clamped against the
    // lines it actually produced, further down.
    let Some((revealed, reverse, card)) = ({
        let Some(session) = app.session.as_ref() else {
            return;
        };
        let Some(card_id) = session.current() else {
            return;
        };
        app.db.deck_by_id(session.deck).and_then(|deck| {
            let card = deck.card(card_id)?.clone();
            Some((session.revealed, deck.reverse, card))
        })
    }) else {
        return;
    };
    let card = &card;

    let [progress_area, card_area, grade_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(5),
        Constraint::Length(3),
    ])
    .areas(area);

    // Prose reads badly across a wide terminal: by the time the eye comes back
    // for the next line it has lost the column. The card keeps to a measure and
    // centres itself; the progress bar follows it so the two stay aligned.
    let card_area = centered(card_area, CARD_MEASURE);
    render_progress(frame, app, centered(progress_area, CARD_MEASURE));

    let lines = if revealed {
        full_card(card, card_text_width(card_area))
    } else if reverse {
        reverse_prompt(card, card_text_width(card_area))
    } else {
        vec![
            Line::from(""),
            card_head(card),
            Line::from(""),
            Line::from(Span::styled(
                "  press space to reveal",
                Style::default().fg(DIM).italic(),
            )),
        ]
    };
    render_scrolling_card(
        frame,
        app,
        lines,
        match (revealed, reverse) {
            (true, _) => "answer",
            (false, true) => "recall the word",
            (false, false) => "recall",
        },
        card_area,
    );

    if revealed {
        render_grades(frame, app, card, grade_area);
    } else {
        let hint = Line::from(vec![
            hint_key("space"),
            Span::styled(" reveal   ", Style::default().fg(DIM)),
            hint_key("m"),
            Span::styled(" mark ★   ", Style::default().fg(DIM)),
            hint_key("e"),
            Span::styled(" edit   ", Style::default().fg(DIM)),
            hint_key("x"),
            Span::styled(" suspend   ", Style::default().fg(DIM)),
            hint_key("S"),
            Span::styled(" bury   ", Style::default().fg(DIM)),
            hint_key("q"),
            Span::styled(" end", Style::default().fg(DIM)),
        ])
        .centered();
        frame.render_widget(
            Paragraph::new(hint).block(Block::default().borders(Borders::NONE)),
            grade_area.inner(Margin::new(0, 1)),
        );
    }
}

/// Columns of text inside a full-size card: borders take 2 columns and the
/// panel's horizontal padding another 4.
pub(super) fn card_text_width(area: Rect) -> usize {
    area.width.saturating_sub(6) as usize
}

/// A card turned over: the word, a rule, then everything behind it under
/// headings. What the review answer and the word view both show.
pub(super) fn full_card(card: &Card, width: usize) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(""), card_head(card), rule(width), Line::from("")];
    lines.extend(card_body(card, width, true));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        scheduling_summary(card),
        Style::default().fg(DIM),
    )));
    lines
}

/// The question side of a reverse card: the meaning, with nothing that spells
/// out the word. The examples stay hidden — they use the word — and so do the
/// reading and the look-alikes; the part of speech is a fair hint.
pub(super) fn reverse_prompt(card: &Card, width: usize) -> Vec<Line<'static>> {
    let mut head = vec![Span::styled("?", Style::default().fg(ACCENT).bold())];
    if !card.pos.is_empty() {
        head.push(Span::styled(
            format!("  {}", card.pos),
            Style::default().fg(DIM).italic(),
        ));
    }
    let mut lines = vec![
        Line::from(""),
        Line::from(head),
        rule(width),
        Line::from(""),
    ];
    lines.extend(wrap_prose(&Prose::definition(), &card.definition, width));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  which word is this? space to reveal",
        Style::default().fg(DIM).italic(),
    )));
    lines
}

/// Draw a card that may be taller than its panel. Every line comes in
/// wrapped already, so the count of lines is the count of rows, and the
/// scroll is clamped against it here, where the size is known.
pub(super) fn render_scrolling_card(
    frame: &mut Frame,
    app: &mut App,
    lines: Vec<Line<'static>>,
    title: &str,
    area: Rect,
) {
    let visible = area.height.saturating_sub(2) as usize;
    let max_scroll = lines.len().saturating_sub(visible) as u16;
    app.card_scroll = app.card_scroll.min(max_scroll);
    let scroll = app.card_scroll;

    let mut block = panel(title).padding(Padding::horizontal(2));
    // What is still off-screen, in the bottom border where it costs no room.
    if max_scroll > 0 {
        let mut marks = Vec::new();
        if scroll > 0 {
            marks.push("↑ k");
        }
        if scroll < max_scroll {
            marks.push("↓ j");
        }
        block = block.title_bottom(
            Line::from(Span::styled(
                format!(" {} ", marks.join("  ")),
                Style::default().fg(ACCENT),
            ))
            .right_aligned(),
        );
    }

    frame.render_widget(Paragraph::new(lines).block(block).scroll((scroll, 0)), area);
}

/// The word view: one card from the browser, in full and at a readable
/// measure, opened with Enter. The same layout as a revealed review card, so
/// a word reads the same wherever you meet it.
pub(super) fn render_card_view(frame: &mut Frame, app: &mut App, area: Rect) {
    let card = app
        .selected_card()
        .and_then(|(deck_id, index)| app.db.deck_by_id(deck_id)?.cards.get(index))
        .cloned();
    let [card_area, hint_area] =
        Layout::vertical([Constraint::Min(5), Constraint::Length(1)]).areas(area);
    let card_area = centered(card_area, CARD_MEASURE);

    let Some(card) = card else {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "  no card here — q to go back",
                Style::default().fg(DIM).italic(),
            ))
            .block(panel("word")),
            card_area,
        );
        return;
    };

    let lines = full_card(&card, card_text_width(card_area));
    let title = format!("word {} of {}", app.browse_sel + 1, app.rows.len());
    render_scrolling_card(frame, app, lines, &title, card_area);

    let hint = Line::from(vec![
        hint_key("j k"),
        Span::styled(" scroll   ", Style::default().fg(DIM)),
        hint_key("J K"),
        Span::styled(" next / previous word   ", Style::default().fg(DIM)),
        hint_key("i"),
        Span::styled(" edit   ", Style::default().fg(DIM)),
        hint_key("m"),
        Span::styled(" status   ", Style::default().fg(DIM)),
        hint_key("p"),
        Span::styled(" say it   ", Style::default().fg(DIM)),
        hint_key("q"),
        Span::styled(" back", Style::default().fg(DIM)),
    ])
    .centered();
    frame.render_widget(Paragraph::new(hint), hint_area);
}

pub(super) fn hint_key(key: &str) -> Span<'static> {
    Span::styled(key.to_string(), Style::default().fg(ACCENT).bold())
}

pub(super) fn render_progress(frame: &mut Frame, app: &App, area: Rect) {
    let Some(session) = app.session.as_ref() else {
        return;
    };
    // Progress is cards seen off, not answers given: a card answered Again and
    // then Good is one card done, and the bar must not read 100% while it is
    // still in the queue.
    let total = session.initial.max(1);
    let ratio = (session.completed as f32 / total as f32).clamp(0.0, 1.0);

    let percent = format!("  {:>3.0}%", ratio * 100.0);
    let done = format!("  {} of {} done", session.completed, session.initial);
    let retrying = session.retrying.len();
    let retry = if retrying > 0 {
        format!("  · {retrying} retrying")
    } else {
        String::new()
    };

    let labels = util::width(&percent) + util::width(&done) + util::width(&retry);
    let width = (area.width as usize).saturating_sub(labels + 2);
    let filled = (ratio * width as f32).round() as usize;
    let bar: String = "━".repeat(filled);
    let rest: String = "━".repeat(width.saturating_sub(filled));

    let line = Line::from(vec![
        Span::raw(" "),
        Span::styled(bar, Style::default().fg(DUE)),
        Span::styled(rest, Style::default().fg(DIM)),
        Span::styled(percent, Style::default().fg(ACCENT).bold()),
        Span::styled(done, Style::default().fg(DIM)),
        Span::styled(retry, Style::default().fg(MARK)),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

pub(super) fn render_grades(frame: &mut Frame, app: &App, card: &Card, area: Rect) {
    match app.db.grading {
        Grading::Binary => render_grades_binary(frame, app, card, area),
        Grading::Sm2 => render_grades_sm2(frame, app, card, area),
    }
}

/// Two buttons: "I know this" and "I don't know".
pub(super) fn render_grades_binary(frame: &mut Frame, app: &App, card: &Card, area: Rect) {
    let preview = card.preview(app.today);
    let good = preview[Grade::Good.index() as usize]
        .map(humanize_days)
        .unwrap_or_else(|| "<1d".to_string());

    let buttons: [(char, &str, String, Color); 2] = [
        ('y', "I know this", format!("next in {good}"), DUE),
        (
            'n',
            "I don't know",
            if card.status.is_set() {
                "again now".to_string()
            } else {
                "again now · marks it ★".to_string()
            },
            WARN,
        ),
    ];

    let columns = Layout::horizontal([Constraint::Ratio(1, 2); 2]).split(area);
    for (i, (key, label, caption, color)) in buttons.into_iter().enumerate() {
        let text = Text::from(vec![
            Line::from(vec![
                Span::styled(
                    format!(" {key} "),
                    Style::default().fg(ON_BADGE).bg(color).bold(),
                ),
                Span::styled(format!("  {label}"), Style::default().fg(color).bold()),
            ])
            .centered(),
            Line::from(Span::styled(caption, Style::default().fg(DIM))).centered(),
        ]);
        frame.render_widget(Paragraph::new(text), columns[i].inner(Margin::new(1, 0)));
    }
}

pub(super) fn render_grades_sm2(frame: &mut Frame, app: &App, card: &Card, area: Rect) {
    let preview = card.preview(app.today);
    let colors = [WARN, MARK, DUE, ACCENT];

    let columns = Layout::horizontal([Constraint::Ratio(1, 4); 4]).split(area);
    for (i, grade) in Grade::ALL.iter().enumerate() {
        let interval = match preview[i] {
            Some(days) => humanize_days(days),
            None => "<1d".to_string(),
        };
        let text = Text::from(vec![
            Line::from(vec![
                Span::styled(
                    format!("{} ", grade.key()),
                    Style::default().fg(ON_BADGE).bg(colors[i]).bold(),
                ),
                Span::styled(
                    format!(" {}", grade.label()),
                    Style::default().fg(colors[i]).bold(),
                ),
            ])
            .centered(),
            Line::from(Span::styled(interval, Style::default().fg(DIM))).centered(),
        ]);
        frame.render_widget(Paragraph::new(text), columns[i].inner(Margin::new(1, 0)));
    }
}
