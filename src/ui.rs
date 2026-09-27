//! All drawing. `render` is the single entry point called each frame.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin, Position, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState, Padding, Paragraph, Wrap,
};

use crate::app::{App, FIELDS, Mode, Scope, Screen};
use crate::model::{Card, CardState, Grade, Grading};
use crate::util::{humanize_days, split_emphasis};

const ACCENT: Color = Color::Cyan;
const DIM: Color = Color::DarkGray;
const NEW: Color = Color::Blue;
const LEARN: Color = Color::Magenta;
const DUE: Color = Color::Green;
const MARK: Color = Color::Yellow;

pub fn render(frame: &mut Frame, app: &mut App) {
    let [header, body, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    render_header(frame, app, header);

    match app.screen {
        Screen::Decks => render_decks(frame, app, body),
        Screen::Browse => render_browse(frame, app, body),
        Screen::Review => render_review(frame, app, body),
        Screen::Stats => render_stats(frame, app, body),
        Screen::Help => render_help(frame, app, body),
    }

    if app.editor.is_some() {
        render_editor(frame, app, body);
    }
    if app.confirm.is_some() {
        render_confirm(frame, app, body);
    }

    render_status(frame, app, status);
}

// -- chrome ---------------------------------------------------------------

fn render_header(frame: &mut Frame, app: &App, area: Rect) {
    let deck = app
        .active_deck()
        .and_then(|i| app.db.decks.get(i))
        .map(|d| d.name.as_str());

    let screen = match app.screen {
        Screen::Decks => "decks",
        Screen::Browse => "browse",
        Screen::Review => "review",
        Screen::Stats => "stats",
        Screen::Help => "help",
    };

    let mut spans = vec![
        Span::styled(" gram ", Style::default().fg(Color::Black).bg(ACCENT).bold()),
        Span::raw(" "),
        Span::styled(screen, Style::default().fg(ACCENT)),
    ];
    if let Some(name) = deck {
        spans.push(Span::styled("  ·  ", Style::default().fg(DIM)));
        spans.push(Span::raw(name.to_string()));
    }
    if let Some(session) = app.session.as_ref() {
        if session.scope == Scope::Marked {
            spans.push(Span::styled("  ★ marked", Style::default().fg(MARK)));
        }
    }

    let right = Line::from(vec![
        Span::styled(app.today.format("%a %d %b").to_string(), Style::default().fg(DIM)),
        Span::raw(" "),
    ])
    .right_aligned();

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
    frame.render_widget(Paragraph::new(right), area);
}

fn render_status(frame: &mut Frame, app: &App, area: Rect) {
    // In `:`/`/` mode the status line becomes the command line.
    if matches!(app.mode, Mode::Command | Mode::Search) {
        let prefix = if app.mode == Mode::Command { ':' } else { '/' };
        let width = area.width.saturating_sub(2) as usize;
        let (visible, offset) = window(app.line.value(), app.line.cursor(), width);
        frame.render_widget(Paragraph::new(format!("{prefix}{visible}")), area);
        let column = area.x + 1 + offset as u16;
        frame.set_cursor_position(Position::new(
            column.min(area.right().saturating_sub(1)),
            area.y,
        ));
        return;
    }

    let mode_style = match app.mode {
        Mode::Insert => Style::default().fg(Color::Black).bg(Color::Green).bold(),
        _ => Style::default().fg(Color::Black).bg(DIM).bold(),
    };

    let mut left = vec![];
    if app.mode == Mode::Insert || app.editor.is_some() {
        left.push(Span::styled(format!(" {} ", app.mode.label()), mode_style));
        left.push(Span::raw(" "));
    }
    let message_style = if app.status.error {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(DIM)
    };
    left.push(Span::styled(app.status.text.clone(), message_style));

    let mut right = String::new();
    if !app.count.is_empty() {
        right.push_str(&app.count);
    }
    if let Some(pending) = app.pending {
        right.push(pending);
    }
    if app.dirty {
        right.push_str(" [+]");
    }
    right.push_str("  ? help ");

    frame.render_widget(Paragraph::new(Line::from(left)), area);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(right, Style::default().fg(DIM))).right_aligned()),
        area,
    );
}

fn panel(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(DIM))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(ACCENT),
        ))
}

// -- deck list -------------------------------------------------------------

fn render_decks(frame: &mut Frame, app: &mut App, area: Rect) {
    if app.db.decks.is_empty() {
        let text = Text::from(vec![
            Line::from(""),
            Line::from("  No decks yet.").bold(),
            Line::from(""),
            Line::from(Span::styled(
                "  :import decks/word_smart.csv Word Smart    load a CSV",
                Style::default().fg(DIM),
            )),
            Line::from(Span::styled(
                "  :deck <name>                               create an empty deck",
                Style::default().fg(DIM),
            )),
            Line::from(Span::styled(
                "  ?                                          keybindings",
                Style::default().fg(DIM),
            )),
        ]);
        frame.render_widget(Paragraph::new(text).block(panel("decks")), area);
        return;
    }

    let today = app.today;
    let items: Vec<ListItem> = app
        .db
        .decks
        .iter()
        .map(|deck| {
            let counts = deck.counts(today);
            let name = format!("{:<28}", truncate(&deck.name, 28));
            let name_style = if counts.pending() == 0 {
                Style::default().fg(DIM)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(vec![
                Span::styled(name, name_style),
                count_span(counts.new, NEW),
                count_span(counts.learning, LEARN),
                count_span(counts.due, DUE),
                Span::styled(
                    format!("   {:>5} cards", counts.total),
                    Style::default().fg(DIM),
                ),
                if counts.marked > 0 {
                    Span::styled(
                        format!("  ★ {}", counts.marked),
                        Style::default().fg(MARK),
                    )
                } else {
                    Span::raw("")
                },
                if counts.suspended > 0 {
                    Span::styled(
                        format!("  {} suspended", counts.suspended),
                        Style::default().fg(DIM),
                    )
                } else {
                    Span::raw("")
                },
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(panel("decks"))
        .highlight_style(Style::default().bg(Color::Indexed(236)).bold())
        .highlight_symbol("▌");

    let mut state = ListState::default();
    state.select(Some(app.decks_sel));
    frame.render_stateful_widget(list, area, &mut state);
}

fn count_span(count: usize, color: Color) -> Span<'static> {
    let style = if count == 0 {
        Style::default().fg(DIM)
    } else {
        Style::default().fg(color).bold()
    };
    Span::styled(format!("{count:>5}"), style)
}

// -- card browser ----------------------------------------------------------

fn render_browse(frame: &mut Frame, app: &mut App, area: Rect) {
    let Some(deck) = app.db.decks.get(app.browse_deck) else {
        frame.render_widget(Paragraph::new("no deck").block(panel("browse")), area);
        return;
    };

    let title = match (app.filter.is_empty(), app.marked_only) {
        (true, false) => format!("{} — {} cards", deck.name, deck.cards.len()),
        (true, true) => format!("{} — {} marked ★", deck.name, app.rows.len()),
        (false, false) => format!(
            "{} — {}/{} matching {:?}",
            deck.name,
            app.rows.len(),
            deck.cards.len(),
            app.filter
        ),
        (false, true) => format!(
            "{} — {} marked ★ matching {:?}",
            deck.name,
            app.rows.len(),
            app.filter
        ),
    };

    let width = area.width.saturating_sub(4) as usize;
    let term_width = 22.min(width / 3);
    let definition_width = width.saturating_sub(term_width + 22);
    let today = app.today;

    let items: Vec<ListItem> = app
        .rows
        .iter()
        .filter_map(|i| deck.cards.get(*i))
        .map(|card| {
            let (state_text, state_color) = match card.state {
                _ if card.suspended => ("susp", DIM),
                CardState::New => ("new", NEW),
                CardState::Learning => ("learn", LEARN),
                CardState::Review => ("rev", DUE),
            };
            let due = if card.suspended || card.state == CardState::New {
                "—".to_string()
            } else {
                let days = (card.due - today).num_days();
                match days {
                    d if d <= 0 => "due".to_string(),
                    d => format!("{d}d"),
                }
            };

            let mut term_style = Style::default().bold();
            if card.suspended {
                term_style = Style::default().fg(DIM).add_modifier(Modifier::CROSSED_OUT);
            }

            ListItem::new(Line::from(vec![
                Span::styled(
                    if card.marked { "★ " } else { "  " },
                    Style::default().fg(MARK),
                ),
                Span::styled(
                    format!("{:<width$}", truncate(&card.term, term_width), width = term_width),
                    term_style,
                ),
                Span::styled(
                    format!(" {:<5}", truncate(&card.pos, 5)),
                    Style::default().fg(DIM).italic(),
                ),
                Span::raw(format!(
                    " {:<width$}",
                    truncate(&card.definition, definition_width),
                    width = definition_width
                )),
                Span::styled(format!(" {state_text:<5}"), Style::default().fg(state_color)),
                Span::styled(format!(" {due:>5}"), Style::default().fg(DIM)),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(panel(&title))
        .highlight_style(Style::default().bg(Color::Indexed(236)))
        .highlight_symbol("▌");

    let mut state = ListState::default();
    if !app.rows.is_empty() {
        state.select(Some(app.browse_sel));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

// -- review ----------------------------------------------------------------

fn render_review(frame: &mut Frame, app: &mut App, area: Rect) {
    let Some(session) = app.session.as_ref() else {
        return;
    };
    let Some(card_id) = session.current() else {
        return;
    };
    let Some(card) = app.db.decks.get(session.deck).and_then(|d| d.card(card_id)) else {
        return;
    };

    let [progress_area, card_area, grade_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(5),
        Constraint::Length(3),
    ])
    .areas(area);

    render_progress(frame, app, progress_area);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));

    // -- question side --
    let mut head = vec![Span::styled(
        card.term.clone(),
        Style::default().fg(ACCENT).bold(),
    )];
    if card.marked {
        head.push(Span::styled("  ★", Style::default().fg(MARK)));
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
    lines.push(Line::from(head));
    lines.push(Line::from(""));

    if session.revealed {
        lines.push(Line::from(Span::styled(
            card.definition.clone(),
            Style::default().fg(Color::White).bold(),
        )));

        let examples: Vec<&str> = card.examples().collect();
        if !examples.is_empty() {
            lines.push(Line::from(""));
            for example in examples {
                lines.push(emphasised_line("  • ", example));
            }
        }

        if !card.notes.is_empty() {
            lines.push(Line::from(""));
            for note in card.notes.lines().filter(|l| !l.trim().is_empty()) {
                lines.push(emphasised_line("  ", note));
            }
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            scheduling_summary(card),
            Style::default().fg(DIM),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "  press space to reveal",
            Style::default().fg(DIM).italic(),
        )));
    }

    let block = panel(if session.revealed { "answer" } else { "recall" })
        .padding(Padding::horizontal(2));
    frame.render_widget(
        Paragraph::new(lines).block(block).wrap(Wrap { trim: false }),
        card_area,
    );

    if session.revealed {
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

fn hint_key(key: &str) -> Span<'static> {
    Span::styled(key.to_string(), Style::default().fg(ACCENT).bold())
}

fn render_progress(frame: &mut Frame, app: &App, area: Rect) {
    let Some(session) = app.session.as_ref() else {
        return;
    };
    let done = session.answered as usize;
    let total = session.initial.max(1);
    let ratio = (done as f32 / total as f32).clamp(0.0, 1.0);

    let width = area.width.saturating_sub(24) as usize;
    let filled = (ratio * width as f32).round() as usize;
    let bar: String = "━".repeat(filled);
    let rest: String = "━".repeat(width.saturating_sub(filled));

    let line = Line::from(vec![
        Span::raw(" "),
        Span::styled(bar, Style::default().fg(ACCENT)),
        Span::styled(rest, Style::default().fg(DIM)),
        Span::styled(
            format!("  {} left", session.remaining()),
            Style::default().fg(DIM),
        ),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn render_grades(frame: &mut Frame, app: &App, card: &Card, area: Rect) {
    match app.db.grading {
        Grading::Binary => render_grades_binary(frame, app, card, area),
        Grading::Sm2 => render_grades_sm2(frame, app, card, area),
    }
}

/// Two buttons: "I know this" and "I don't know".
fn render_grades_binary(frame: &mut Frame, app: &App, card: &Card, area: Rect) {
    let preview = card.preview(app.today);
    let good = preview[Grade::Good.index() as usize]
        .map(humanize_days)
        .unwrap_or_else(|| "<1d".to_string());

    let buttons: [(char, &str, String, Color); 2] = [
        ('y', "I know this", format!("next in {good}"), DUE),
        (
            'n',
            "I don't know",
            if card.marked {
                "again now".to_string()
            } else {
                "again now · marks it ★".to_string()
            },
            Color::Red,
        ),
    ];

    let columns = Layout::horizontal([Constraint::Ratio(1, 2); 2]).split(area);
    for (i, (key, label, caption, color)) in buttons.into_iter().enumerate() {
        let text = Text::from(vec![
            Line::from(vec![
                Span::styled(
                    format!(" {key} "),
                    Style::default().fg(Color::Black).bg(color).bold(),
                ),
                Span::styled(format!("  {label}"), Style::default().fg(color).bold()),
            ])
            .centered(),
            Line::from(Span::styled(caption, Style::default().fg(DIM))).centered(),
        ]);
        frame.render_widget(Paragraph::new(text), columns[i].inner(Margin::new(1, 0)));
    }
}

fn render_grades_sm2(frame: &mut Frame, app: &App, card: &Card, area: Rect) {
    let preview = card.preview(app.today);
    let colors = [Color::Red, Color::Yellow, DUE, ACCENT];

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
                    Style::default().fg(Color::Black).bg(colors[i]).bold(),
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

fn scheduling_summary(card: &Card) -> String {
    format!(
        "  {} · ease {:.2} · reps {} · lapses {}",
        card.state.label(),
        card.ease,
        card.reps,
        card.lapses
    )
}

/// Render `*emphasised*` runs from the importer in italic accent.
fn emphasised_line<'a>(prefix: &'a str, text: &'a str) -> Line<'a> {
    let mut spans = vec![Span::raw(prefix)];
    for (run, emphasised) in split_emphasis(text) {
        if emphasised {
            spans.push(Span::styled(
                run,
                Style::default().fg(Color::Yellow).italic(),
            ));
        } else {
            spans.push(Span::styled(run, Style::default().fg(Color::Gray)));
        }
    }
    Line::from(spans)
}

// -- statistics ------------------------------------------------------------

fn render_stats(frame: &mut Frame, app: &mut App, area: Rect) {
    let history = app.review_history(14);
    let forecast = app.due_forecast(14);
    let retention = app.retention();

    let total_cards: usize = app.db.decks.iter().map(|d| d.cards.len()).sum();
    let reviewed_today = history.last().map(|(_, n)| *n).unwrap_or(0);
    let mature = app
        .db
        .decks
        .iter()
        .flat_map(|d| d.cards.iter())
        .filter(|c| c.interval >= 21)
        .count();
    let marked = app
        .db
        .decks
        .iter()
        .flat_map(|d| d.cards.iter())
        .filter(|c| c.marked && !c.suspended)
        .count();

    let mut lines: Vec<Line> = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  cards      ", Style::default().fg(DIM)),
            Span::styled(total_cards.to_string(), Style::default().bold()),
            Span::styled("     mature (21d+) ", Style::default().fg(DIM)),
            Span::styled(mature.to_string(), Style::default().bold()),
        ]),
        Line::from(vec![
            Span::styled("  today      ", Style::default().fg(DIM)),
            Span::styled(
                format!("{reviewed_today} answers"),
                Style::default().bold(),
            ),
            Span::styled("     retention ", Style::default().fg(DIM)),
            Span::styled(
                match retention {
                    Some(rate) => format!("{:.0}%", rate * 100.0),
                    None => "—".to_string(),
                },
                Style::default().bold(),
            ),
        ]),
        Line::from(vec![
            Span::styled("  marked     ", Style::default().fg(DIM)),
            Span::styled(format!("{marked} ★"), Style::default().fg(MARK).bold()),
            Span::styled("     grading ", Style::default().fg(DIM)),
            Span::styled(app.db.grading.label(), Style::default().bold()),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  reviews, last 14 days",
            Style::default().fg(ACCENT),
        )),
    ];
    lines.extend(bar_rows(&history, Color::Green));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  due, next 14 days",
        Style::default().fg(ACCENT),
    )));
    lines.extend(bar_rows(&forecast, Color::Blue));

    frame.render_widget(
        Paragraph::new(lines).block(panel("stats")).scroll((app.help_scroll, 0)),
        area,
    );
}

fn bar_rows(data: &[(chrono::NaiveDate, u64)], color: Color) -> Vec<Line<'static>> {
    let peak = data.iter().map(|(_, n)| *n).max().unwrap_or(0).max(1);
    data.iter()
        .map(|(date, count)| {
            let width = ((*count as f32 / peak as f32) * 40.0).round() as usize;
            Line::from(vec![
                Span::styled(
                    format!("  {}  ", date.format("%a %d")),
                    Style::default().fg(DIM),
                ),
                Span::styled("█".repeat(width), Style::default().fg(color)),
                Span::styled(
                    if *count > 0 {
                        format!(" {count}")
                    } else {
                        String::new()
                    },
                    Style::default().fg(DIM),
                ),
            ])
        })
        .collect()
}

// -- editor overlay --------------------------------------------------------

fn render_editor(frame: &mut Frame, app: &App, area: Rect) {
    let Some(editor) = app.editor.as_ref() else {
        return;
    };
    // Six two-line fields, a hint line, borders and top padding.
    let popup = centered_fixed(88, (FIELDS.len() * 2 + 4) as u16, area);
    frame.render_widget(Clear, popup);

    let title = match editor.target {
        crate::app::EditTarget::New { .. } => "new card",
        crate::app::EditTarget::Existing { .. } => "edit card",
    };
    let block = panel(title)
        .border_style(Style::default().fg(ACCENT))
        .padding(Padding::new(1, 1, 1, 0));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let [fields_area, hint_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);
    let rows = Layout::vertical([Constraint::Length(2); 6]).split(fields_area);

    const LABEL_WIDTH: u16 = 12;

    for (i, name) in FIELDS.iter().enumerate() {
        let row = rows[i];
        if row.height == 0 {
            continue;
        }
        let focused = i == editor.field;
        let label_style = if focused {
            Style::default().fg(ACCENT).bold()
        } else {
            Style::default().fg(DIM)
        };

        // Long values scroll horizontally so the cursor stays on screen.
        let width = row.width.saturating_sub(LABEL_WIDTH) as usize;
        let field = &editor.fields[i];
        let (visible, offset) = if focused {
            window(field.value(), field.cursor(), width)
        } else {
            window(field.value(), 0, width)
        };

        let line = Line::from(vec![
            Span::styled(format!("{name:>10}  "), label_style),
            Span::styled(
                visible,
                if focused {
                    Style::default()
                } else {
                    Style::default().fg(Color::Gray)
                },
            ),
        ]);
        frame.render_widget(Paragraph::new(line), row);

        if focused {
            let column = row.x + LABEL_WIDTH + offset as u16;
            frame.set_cursor_position(Position::new(
                column.min(row.right().saturating_sub(1)),
                row.y,
            ));
        }
    }

    let hint = Line::from(Span::styled(
        "Tab/j/k field   i insert   Esc normal   ZZ save   q cancel   \\n = newline",
        Style::default().fg(DIM),
    ))
    .centered();
    frame.render_widget(Paragraph::new(hint), hint_area);
}

fn render_confirm(frame: &mut Frame, app: &App, area: Rect) {
    let Some(confirm) = app.confirm.as_ref() else {
        return;
    };
    // Wide enough for the prompt to wrap to at most two lines.
    let width = 64.min(area.width);
    let block = panel("confirm")
        .border_style(Style::default().fg(Color::Yellow))
        .padding(Padding::horizontal(1));
    let text = Paragraph::new(Line::from(confirm.prompt.clone()).centered())
        .block(block)
        .wrap(Wrap { trim: true });

    let lines = wrapped_lines(&confirm.prompt, width.saturating_sub(4) as usize);
    let popup = centered_fixed(width, lines.saturating_add(2), area);

    frame.render_widget(Clear, popup);
    frame.render_widget(text, popup);
}

// -- help ------------------------------------------------------------------

const HELP: &str = "\
MOVING AROUND
  j / k          down / up                 gg / G      first / last
  5j             down five                 :42         jump to line 42
  Ctrl-d/u       half page                 Ctrl-f/b    full page
  h / l          back / forward            Enter       open
  q / Esc        back one screen           ZZ          save and quit
  ?              this help                 S           statistics

DECKS
  s              study the selected deck   a           add a card
  Enter / l      browse its cards          dd          delete deck (confirms)

BROWSING
  /text          filter cards              n / N       next / previous match
  :noh           clear filter and marks    i / Enter   edit the card
  a / o          add a card                dd          delete card
  u              undo a delete             x           suspend / unsuspend
  m              mark / unmark ★           :marked     show only marked

REVIEWING
  space          reveal the answer         space       again: grade Good
  y              I know this               n           I don't know
  1 2 3 4        Again Hard Good Easy      u           undo the last answer
  m              mark / unmark ★           e           edit this card
  x              suspend it                S           bury until later
  q              end the session

THE STUDY PILE
  Mark a word with m and it joins the pile of words you still have to learn.
  Answering \"I don't know\" marks it for you; m unmarks it once it sticks.

  M              drill the marked words    :study marked   the same thing
  :marked        browse only marked        :noh            show everything again

  A marked drill ignores due dates — it pulls those words forward on purpose.
  Answers still count, so the schedule keeps moving.

GRADING MODES
  :set grading=binary    y \"I know this\" and n \"I don't know\"   (default)
  :set grading=sm2       the full Again / Hard / Good / Easy row

  Both modes drive the same scheduler: know = Good, don't know = Again.
  1 2 3 4 keep working in either mode when you want the finer grades.

EDITING A CARD
  Tab / j / k    next / previous field     i a I A     insert / append
  Esc            leave insert mode         h l 0 $ w b cursor motions
  x D C          delete char / to end      dd          clear the field
  ZZ / Ctrl-s    save the card             q / Esc     cancel
  \\n             literal newline in the Example and Notes fields

COMMANDS
  :w             save the collection       :q          back / quit
  :wq  :x        save and quit             :q!         quit, discarding changes
  :deck NAME     create a deck             :rename N   rename this deck
  :import FILE [DECK]                      import a CSV or TSV
  :export FILE                             export this deck as CSV
  :set new=20    new cards per day         :forget     reset this deck's schedule
  :set grading=binary|sm2                  :study marked   drill the study pile
  :study :browse :marked :stats :help

SCHEDULING
  gram uses SM-2. Again resets the card and shows it again this session;
  Hard, Good and Easy push it further out, scaled by the card's ease factor.
  Each deck introduces at most `new_per_day` unseen cards per day.

CSV FORMAT
  Recognised headers, in any order:
      term, pron, pos, definition, example, notes
  Without a header row, columns are read as term, definition, example.
  Re-importing a file updates existing terms instead of duplicating them,
  so your scheduling survives a corrected export.
";

fn render_help(frame: &mut Frame, app: &mut App, area: Rect) {
    let lines: Vec<Line> = HELP
        .lines()
        .map(|line| {
            if !line.starts_with(' ') && !line.is_empty() {
                Line::from(Span::styled(
                    line.to_string(),
                    Style::default().fg(ACCENT).bold(),
                ))
            } else {
                // Colour the key column.
                let split = line
                    .char_indices()
                    .nth(17)
                    .map(|(i, _)| i)
                    .unwrap_or(line.len());
                let (keys, rest) = line.split_at(split);
                Line::from(vec![
                    Span::styled(keys.to_string(), Style::default().fg(Color::Yellow)),
                    Span::styled(rest.to_string(), Style::default().fg(Color::Gray)),
                ])
            }
        })
        .collect();

    let total = lines.len() as u16;
    let visible = area.height.saturating_sub(2);
    let max_scroll = total.saturating_sub(visible);
    app.help_scroll = app.help_scroll.min(max_scroll);

    frame.render_widget(
        Paragraph::new(lines)
            .block(panel("help — q to close"))
            .scroll((app.help_scroll, 0)),
        area,
    );
}

// -- helpers ---------------------------------------------------------------

/// A `width` x `height` rect centred in `area`, shrunk to fit if need be.
fn centered_fixed(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

/// Height `text` needs when word-wrapped into `width` columns.
fn wrapped_lines(text: &str, width: usize) -> u16 {
    if width == 0 {
        return 1;
    }
    let mut lines = 1u16;
    let mut column = 0usize;
    for word in text.split_whitespace() {
        let len = word.chars().count();
        if column == 0 {
            column = len;
        } else if column + 1 + len <= width {
            column += 1 + len;
        } else {
            lines += 1;
            column = len;
        }
    }
    lines
}

/// The slice of `text` that keeps `cursor` visible in `width` columns.
/// Returns the visible text and the cursor's column within it.
fn window(text: &str, cursor: usize, width: usize) -> (String, usize) {
    if width == 0 {
        return (String::new(), 0);
    }
    let chars: Vec<char> = text.chars().collect();
    let start = cursor.saturating_sub(width.saturating_sub(1));
    let end = (start + width).min(chars.len());
    (chars[start..end].iter().collect(), cursor - start)
}

/// Truncate to `width` display columns, adding an ellipsis when it cuts.
fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    if width <= 1 {
        return "…".to_string();
    }
    let mut out: String = text.chars().take(width - 1).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
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
    fn centered_fixed_shrinks_to_fit() {
        let area = Rect { x: 0, y: 0, width: 40, height: 10 };
        let popup = centered_fixed(80, 20, area);
        assert_eq!((popup.width, popup.height), (40, 10));

        let popup = centered_fixed(20, 4, area);
        assert_eq!((popup.x, popup.y, popup.width, popup.height), (10, 3, 20, 4));
    }

    #[test]
    fn truncate_counts_characters_not_bytes() {
        assert_eq!(truncate("naïveté", 7), "naïveté");
        assert_eq!(truncate("naïveté", 4), "naï…");
    }
}
