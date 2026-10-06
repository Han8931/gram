//! All drawing. `render` is the single entry point called each frame.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin, Position, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState, Padding, Paragraph, Wrap,
};

use crate::app::{App, Completion, FIELDS, Mode, Scope, Screen, StatusFilter};
use crate::model::{Card, CardState, DeckStatus, Grade, Grading, Study};
use crate::util::{self, pad, truncate};
use crate::util::{humanize_days, split_emphasis};

// One palette, used by role rather than by name, so the screens stay
// consistent with each other.
/// Chrome: titles, focus, the program's own voice.
const ACCENT: Color = Color::Cyan;
/// Secondary text: labels, captions, anything you read second.
const DIM: Color = Color::DarkGray;
/// Body text. Brighter than DIM, calmer than white.
const TEXT: Color = Color::Gray;
/// Emphasis inside body text — a heading, a term, the answer.
const BRIGHT: Color = Color::White;
/// Card states, shared by every count, badge and column that names them.
const NEW: Color = Color::LightBlue;
const LEARN: Color = Color::LightMagenta;
const DUE: Color = Color::LightGreen;
/// The ★ study pile, and emphasised words inside example sentences.
const MARK: Color = Color::Yellow;
/// Something needs attention: overdue cards, "I don't know", errors.
const WARN: Color = Color::LightRed;
/// Selected row, and the header strip above a table.
const SEL_BG: Color = Color::Indexed(237);
const HEAD_BG: Color = Color::Indexed(234);

/// Widest the review card gets, borders included. Roughly 76 columns of text,
/// which is about as long a line as prose stays readable at.
const CARD_MEASURE: u16 = 82;

fn status_color(status: DeckStatus) -> Color {
    match status {
        DeckStatus::ReviewNeeded => DUE,
        DeckStatus::NewCards => NEW,
        DeckStatus::UpToDate => DIM,
        DeckStatus::Empty => DIM,
    }
}

/// A word's own status gets its own colour, used everywhere it appears.
fn study_color(status: Study) -> Color {
    match status {
        Study::None => DIM,
        Study::Review => ACCENT,
        Study::Marked => MARK,
        Study::Test => LEARN,
    }
}

/// `★ marked`, for the browser column and the review screen.
fn study_span(status: Study) -> Span<'static> {
    let style = Style::default().fg(study_color(status));
    match status {
        Study::None => Span::styled("—", style),
        other => Span::styled(format!("{} {}", other.glyph(), other.label()), style.bold()),
    }
}

/// A dim strip of column labels above a list.
fn table_header(labels: Line<'static>) -> Paragraph<'static> {
    Paragraph::new(labels).style(Style::default().bg(HEAD_BG))
}

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

    // Tab's candidate list sits directly above the command line, Vim's
    // wildmenu, and goes away with the completion that produced it.
    if let Some(completion) = app.completion.as_ref() {
        let strip = Rect {
            y: status.y.saturating_sub(1),
            height: 1,
            ..status
        };
        render_wildmenu(frame, completion, strip);
    }
}

/// What Tab is offering, with the candidate currently showing picked out.
fn render_wildmenu(frame: &mut Frame, completion: &Completion, area: Rect) {
    let mut spans = Vec::new();
    for (index, candidate) in completion.candidates.iter().enumerate() {
        let style = if completion.index == Some(index) {
            Style::default().fg(Color::Black).bg(ACCENT).bold()
        } else {
            Style::default().fg(TEXT)
        };
        spans.push(Span::styled(format!(" {} ", menu_label(candidate)), style));
    }

    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(HEAD_BG)),
        area,
    );
}

/// Paths are listed by their last segment: the directory part is already on
/// the command line, and repeating it leaves room for nothing else.
fn menu_label(candidate: &str) -> String {
    let directory = candidate.ends_with('/');
    let name = candidate
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(candidate);
    if directory {
        format!("{name}/")
    } else {
        name.to_string()
    }
}

// -- chrome ---------------------------------------------------------------

fn render_header(frame: &mut Frame, app: &App, area: Rect) {
    let deck = app
        .active_deck()
        .and_then(|id| app.db.deck_by_id(id))
        .map(|d| d.name.as_str());

    let screen = match app.screen {
        Screen::Decks => "decks",
        Screen::Browse => "browse",
        Screen::Review => "review",
        Screen::Stats => "stats",
        Screen::Help => "help",
    };

    let drilling = app
        .session
        .as_ref()
        .filter(|s| matches!(s.scope, Scope::Flagged(_)))
        .map(|s| format!("  drilling {}", s.scope.label()));

    // What the active deck wants from you, carried on every screen.
    let mut right_spans = Vec::new();
    if let Some(counts) = app
        .active_deck()
        .and_then(|id| app.db.deck_by_id(id))
        .map(|d| d.counts(app.today))
    {
        let status = counts.status();
        right_spans.push(Span::styled(
            status.label().to_string(),
            Style::default().fg(status_color(status)).bold(),
        ));
        if counts.pending() > 0 {
            right_spans.push(Span::styled(
                format!(" ({})", counts.pending()),
                Style::default().fg(status_color(status)),
            ));
        }
        right_spans.push(Span::styled("  ·  ", Style::default().fg(DIM)));
    }
    right_spans.push(Span::styled(
        app.today.format("%a %d %b").to_string(),
        Style::default().fg(DIM),
    ));
    right_spans.push(Span::raw(" "));

    // Both halves used to be drawn into the whole strip, so a long deck name
    // and the status badge wrote over each other. Give each its own columns,
    // and shorten the deck name to fit rather than running it into the status.
    let needed: usize = right_spans.iter().map(|s| util::width(&s.content)).sum();
    let [left_area, right_area] = Layout::horizontal([
        Constraint::Min(0),
        Constraint::Length(needed.min(area.width as usize) as u16),
    ])
    .areas(area);

    let mut spans = vec![
        Span::styled(
            " gram ",
            Style::default().fg(Color::Black).bg(ACCENT).bold(),
        ),
        Span::raw(" "),
        Span::styled(screen, Style::default().fg(ACCENT)),
    ];
    if let Some(name) = deck {
        let used: usize = spans.iter().map(|s| util::width(&s.content)).sum();
        let drill = drilling.as_ref().map_or(0, |d| util::width(d));
        // Two columns of air before the status, and five for the separator.
        let room = (left_area.width as usize).saturating_sub(used + drill + 7);
        spans.push(Span::styled("  ·  ", Style::default().fg(DIM)));
        spans.push(Span::styled(
            truncate(name, room),
            Style::default().fg(BRIGHT).bold(),
        ));
    }
    if let Some(drill) = drilling {
        spans.push(Span::styled(drill, Style::default().fg(MARK)));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), left_area);
    frame.render_widget(
        Paragraph::new(Line::from(right_spans).right_aligned()),
        right_area,
    );
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
        Mode::Insert => Style::default().fg(Color::Black).bg(DUE).bold(),
        _ => Style::default().fg(Color::Black).bg(DIM).bold(),
    };

    let mut left = vec![];
    if app.mode == Mode::Insert || app.editor.is_some() {
        left.push(Span::styled(format!(" {} ", app.mode.label()), mode_style));
        left.push(Span::raw(" "));
    }
    let message_style = if app.status.error {
        Style::default().fg(WARN).bold()
    } else {
        Style::default().fg(TEXT)
    };
    left.push(Span::styled(app.status.text.clone(), message_style));

    // The half-typed count and operator, Vim's bottom-right corner.
    let mut right = vec![];
    let keys = format!("{}{}", app.count, app.pending.unwrap_or(' '));
    if !keys.trim().is_empty() {
        right.push(Span::styled(keys, Style::default().fg(ACCENT).bold()));
        right.push(Span::raw(" "));
    }
    if app.dirty {
        right.push(Span::styled(" [+]", Style::default().fg(MARK).bold()));
    }
    right.push(Span::styled("  ? help ", Style::default().fg(DIM)));

    frame.render_widget(Paragraph::new(Line::from(left)), area);
    frame.render_widget(Paragraph::new(Line::from(right).right_aligned()), area);
}

/// Centre `area` horizontally, capped at `width` columns. Narrower terminals
/// keep every column they have.
fn centered(area: Rect, width: u16) -> Rect {
    if area.width <= width {
        return area;
    }
    Rect {
        x: area.x + (area.width - width) / 2,
        width,
        ..area
    }
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
                Span::styled(
                    format!("  {}", status.label()),
                    Style::default().fg(status_color(status)).bold(),
                ),
                if counts.suspended > 0 {
                    Span::styled(
                        format!("  ({} suspended)", counts.suspended),
                        Style::default().fg(DIM),
                    )
                } else {
                    Span::raw("")
                },
            ]))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(Style::default().bg(SEL_BG).bold())
        .highlight_symbol("▌");

    let mut state = ListState::default();
    state.select(Some(app.decks_sel));
    frame.render_stateful_widget(list, rows_area, &mut state);
}

fn count_span(count: usize, color: Color) -> Span<'static> {
    if count == 0 {
        // A dot reads as "nothing here" far faster than a zero does.
        Span::styled(format!("{:>6}", "·"), Style::default().fg(DIM))
    } else {
        Span::styled(format!("{count:>6}"), Style::default().fg(color).bold())
    }
}

// -- card browser ----------------------------------------------------------

/// Rows the browser needs before it shows a preview under the list, and the
/// rows the preview takes. Under the list rather than beside it: the pane gets
/// the full width for prose, and the list keeps its columns.
const PREVIEW_BELOW: u16 = 22;
const PREVIEW_HEIGHT: u16 = 10;

/// Where the browser's preview pane goes: under the list when there are rows
/// to spare, nowhere when there are not.
fn preview_split(area: Rect) -> (Rect, Option<Rect>) {
    if area.height >= PREVIEW_BELOW {
        let [list, preview] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(PREVIEW_HEIGHT)]).areas(area);
        (list, Some(preview))
    } else {
        (area, None)
    }
}

fn render_browse(frame: &mut Frame, app: &mut App, area: Rect) {
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
    let definition_width = width.saturating_sub(term_width + FIXED);
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
            } else {
                Style::default().fg(BRIGHT).bold()
            };

            // The word's own status, as a column you can actually see.
            let status_text = match card.status {
                Study::None => "—".to_string(),
                other => format!("{} {}", other.glyph(), other.label()),
            };

            ListItem::new(Line::from(vec![
                Span::styled(pad(&card.term, term_width), term_style),
                Span::styled(
                    format!(" {}", pad(&card.pos, 4)),
                    Style::default().fg(DIM).italic(),
                ),
                Span::styled(
                    format!(" {}", pad(&card.definition, definition_width)),
                    Style::default().fg(TEXT),
                ),
                Span::styled(
                    format!(" {}", pad(&status_text, STATUS_WIDTH)),
                    Style::default().fg(study_color(card.status)).bold(),
                ),
                Span::styled(
                    format!(" {state_text:<5}"),
                    Style::default().fg(state_color),
                ),
                Span::styled(format!(" {due:>4}"), Style::default().fg(due_color).bold()),
            ]))
        })
        .collect();

    frame.render_widget(
        table_header(Line::from(vec![
            Span::styled(
                format!(" {:<width$}", "TERM", width = term_width),
                Style::default().fg(DIM).bold(),
            ),
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
        ])),
        head_area,
    );

    let list = List::new(items)
        .highlight_style(Style::default().bg(SEL_BG))
        .highlight_symbol("▌");

    let mut state = ListState::default();
    if !app.rows.is_empty() {
        state.select(Some(app.browse_sel));
    }
    frame.render_stateful_widget(list, rows_area, &mut state);

    if let Some(preview_area) = preview_area {
        render_preview(frame, app, preview_area);
    }
}

/// The selected word in full, beside or below the list.
fn render_preview(frame: &mut Frame, app: &App, area: Rect) {
    let mut block = panel("card").padding(Padding::horizontal(1));
    let width = area.width.saturating_sub(4) as usize;

    let card = app
        .selected_card()
        .and_then(|(deck_id, index)| app.db.deck_by_id(deck_id).and_then(|d| d.cards.get(index)));

    let (lines, rows) = match card {
        Some(card) => {
            let mut lines = vec![card_head(card), Line::from("")];
            lines.extend(card_body(card, width));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                scheduling_summary(card),
                Style::default().fg(DIM),
            )));
            // Everything bar the definition is already wrapped to `width`.
            let rows =
                lines.len() + wrapped_lines(&card.definition, width).saturating_sub(1) as usize;
            (lines, rows)
        }
        None => (
            vec![Line::from(Span::styled(
                "nothing selected",
                Style::default().fg(DIM).italic(),
            ))],
            1,
        ),
    };

    // A glance, not a reader: say when there is more and leave it at that.
    if rows > area.height.saturating_sub(2) as usize {
        block = block.title_bottom(
            Line::from(Span::styled(" … ", Style::default().fg(DIM))).right_aligned(),
        );
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

// -- a card, drawn ---------------------------------------------------------

/// The word itself: term, status, pronunciation, part of speech.
fn card_head(card: &Card) -> Line<'static> {
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

/// Everything behind the word: definition, examples, notes. Shared by the
/// review card and the browser's preview, so they read the same.
fn card_body(card: &Card, width: usize) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled(
        card.definition.clone(),
        Style::default().fg(BRIGHT).bold(),
    ))];

    // One blank line ahead of each example, so a run of them reads as
    // separate sentences rather than a single grey block.
    let example = Prose::example();
    for text in card.examples() {
        lines.push(Line::from(""));
        lines.extend(wrap_prose(&example, text, width));
    }

    if !card.notes.is_empty() {
        let note = Prose::note();
        lines.push(Line::from(""));
        for text in card.notes.lines().filter(|l| !l.trim().is_empty()) {
            lines.extend(wrap_prose(&note, text, width));
        }
    }

    lines
}

// -- review ----------------------------------------------------------------

fn render_review(frame: &mut Frame, app: &mut App, area: Rect) {
    // Taken by value so the card's scroll offset can be clamped against the
    // lines it actually produced, further down.
    let Some((revealed, card)) = ({
        let Some(session) = app.session.as_ref() else {
            return;
        };
        let Some(card_id) = session.current() else {
            return;
        };
        app.db
            .deck_by_id(session.deck)
            .and_then(|d| d.card(card_id))
            .map(|card| (session.revealed, card.clone()))
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

    // Borders take 2 columns and the panel's horizontal padding another 4.
    let text_width = card_area.width.saturating_sub(6) as usize;

    let mut lines: Vec<Line> = vec![Line::from(""), card_head(card), Line::from("")];

    if revealed {
        lines.extend(card_body(card, text_width));
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

    // A card with several long examples outgrows the panel, so it scrolls.
    // Every line above is pre-wrapped bar the definition, which the paragraph
    // wraps itself — measure that one so the clamp counts real rows.
    let rows = lines.len() + wrapped_lines(&card.definition, text_width).saturating_sub(1) as usize;
    let visible = card_area.height.saturating_sub(2) as usize;
    let max_scroll = rows.saturating_sub(visible) as u16;
    app.card_scroll = app.card_scroll.min(max_scroll);
    let scroll = app.card_scroll;

    let mut block =
        panel(if revealed { "answer" } else { "recall" }).padding(Padding::horizontal(2));
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

    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
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

fn hint_key(key: &str) -> Span<'static> {
    Span::styled(key.to_string(), Style::default().fg(ACCENT).bold())
}

fn render_progress(frame: &mut Frame, app: &App, area: Rect) {
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

/// Split text into whitespace-separated words, each carrying the pieces it is
/// made of and whether each piece sat inside `*emphasis*` markers. A word is
/// kept whole even when a marker falls inside it, as in `un*abash*edly`.
fn emphasised_words(text: &str) -> Vec<Vec<(String, bool)>> {
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
struct Prose {
    marker: Span<'static>,
    continuation: Span<'static>,
    body: Style,
    emphasis: Style,
}

impl Prose {
    /// An example sentence: an accented bullet, body text, and the target word
    /// picked out so the eye lands on it first.
    fn example() -> Self {
        Self {
            marker: Span::styled("  • ", Style::default().fg(ACCENT)),
            continuation: Span::raw("    "),
            body: Style::default().fg(TEXT),
            emphasis: Style::default().fg(MARK).bold().italic(),
        }
    }

    /// A follow-on note. Dimmed, and behind a gutter rule on every line, so it
    /// reads as an aside instead of one more example.
    fn note() -> Self {
        let gutter = Style::default().fg(DIM);
        Self {
            marker: Span::styled("  │ ", gutter),
            continuation: Span::styled("  │ ", gutter),
            body: gutter,
            emphasis: Style::default().fg(MARK).italic(),
        }
    }
}

/// Word-wrap `text` into `width` columns behind the block's marker, giving
/// continuation lines a hanging indent so a bullet's text stays in one column.
/// Words the importer wrapped in `*` are highlighted.
fn wrap_prose(prose: &Prose, text: &str, width: usize) -> Vec<Line<'static>> {
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

// -- statistics ------------------------------------------------------------

/// A rate as a whole percentage, or an em dash when there is nothing to rate.
fn percent(rate: Option<f32>) -> String {
    match rate {
        Some(rate) => format!("{:.0}%", rate * 100.0),
        None => "—".to_string(),
    }
}

fn render_stats(frame: &mut Frame, app: &mut App, area: Rect) {
    let history = app.review_history(14);
    let forecast = app.due_forecast(14);
    let accuracy = app.answer_accuracy();
    let retention = app.review_retention();

    let total_cards: usize = app.db.decks.iter().map(|d| d.cards.len()).sum();
    let reviewed_today = history.last().map(|(_, n)| *n).unwrap_or(0);
    let mature = app
        .db
        .decks
        .iter()
        .flat_map(|d| d.cards.iter())
        .filter(|c| c.interval >= 21)
        .count();
    let by_status = |status: Study| {
        app.db
            .decks
            .iter()
            .flat_map(|d| d.cards.iter())
            .filter(|c| !c.suspended && c.status == status)
            .count()
    };

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
            Span::styled(format!("{reviewed_today} answers"), Style::default().bold()),
            Span::styled("     accuracy  ", Style::default().fg(DIM)),
            Span::styled(percent(accuracy), Style::default().bold()),
        ]),
        // Two different questions: how the answering went, and how much of it
        // stuck by the time the card came round again.
        Line::from(vec![
            Span::styled("  retention  ", Style::default().fg(DIM)),
            Span::styled(percent(retention), Style::default().bold()),
            Span::styled(
                "     scheduled reviews, first try",
                Style::default().fg(DIM),
            ),
        ]),
        Line::from(vec![
            Span::styled("  status     ", Style::default().fg(DIM)),
            Span::styled(
                format!("{} ◆ review", by_status(Study::Review)),
                Style::default().fg(study_color(Study::Review)).bold(),
            ),
            Span::styled("   ", Style::default()),
            Span::styled(
                format!("{} ★ marked", by_status(Study::Marked)),
                Style::default().fg(study_color(Study::Marked)).bold(),
            ),
            Span::styled("   ", Style::default()),
            Span::styled(
                format!("{} ▲ test", by_status(Study::Test)),
                Style::default().fg(study_color(Study::Test)).bold(),
            ),
        ]),
        Line::from(vec![
            Span::styled("  grading    ", Style::default().fg(DIM)),
            Span::styled(app.db.grading.label(), Style::default().bold()),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  reviews, last 14 days",
            Style::default().fg(ACCENT),
        )),
    ];
    lines.extend(bar_rows(&history, DUE));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  due, next 14 days",
        Style::default().fg(ACCENT),
    )));
    lines.extend(bar_rows(&forecast, NEW));

    // Clamp the scroll the same way the help screen does, so `j` cannot run
    // the page off into blank space.
    let total = lines.len() as u16;
    let visible = area.height.saturating_sub(2);
    app.help_scroll = app.help_scroll.min(total.saturating_sub(visible));

    frame.render_widget(
        Paragraph::new(lines)
            .block(panel("stats — q to close"))
            .scroll((app.help_scroll, 0)),
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

/// Rows each field gets: what its text needs, within reason, shrinking the
/// greediest field first when the form will not fit. Term, Pron and Pos never
/// need more than a line; Example and Notes get room to be read instead of
/// trailing off the edge.
fn field_heights(available: u16, needs: [u16; 6]) -> [u16; 6] {
    const CEILING: [u16; 6] = [1, 1, 1, 3, 8, 8];
    // One blank row between fields.
    let gaps = FIELDS.len() as u16 - 1;

    let mut heights = [1u16; 6];
    for (height, (need, ceiling)) in heights.iter_mut().zip(needs.iter().zip(CEILING)) {
        *height = (*need).clamp(1, ceiling);
    }

    while heights.iter().sum::<u16>() + gaps > available {
        let Some(biggest) = (0..heights.len())
            .filter(|&i| heights[i] > 1)
            .max_by_key(|&i| heights[i])
        else {
            break;
        };
        heights[biggest] -= 1;
    }
    heights
}

/// A field's value laid out for display: the `\n` the editor shows for a
/// newline breaks the line, anything wider than `width` columns word-wraps,
/// and each display line remembers which cluster of the value it starts at —
/// which is what puts the cursor in the right place.
fn wrap_field(value: &str, width: usize) -> Vec<(String, usize)> {
    let width = width.max(1);
    // Clusters, not characters: the cursor counts in the same units, and a
    // combining accent belongs to the letter it sits on.
    let parts: Vec<&str> = util::clusters(value).collect();

    // Split into words, remembering where each one starts, with the editor's
    // `\n` as an explicit break.
    enum Piece {
        Word {
            at: usize,
            text: String,
        },
        /// Break, and the character the next line starts at.
        Break(usize),
    }
    let mut pieces: Vec<Piece> = Vec::new();
    let mut word = String::new();
    let mut word_at = 0usize;
    let mut index = 0usize;
    let flush = |word: &mut String, at: usize, pieces: &mut Vec<Piece>| {
        if !word.is_empty() {
            pieces.push(Piece::Word {
                at,
                text: std::mem::take(word),
            });
        }
    };
    while index < parts.len() {
        if parts[index] == "\\" && parts.get(index + 1) == Some(&"n") {
            flush(&mut word, word_at, &mut pieces);
            index += 2;
            pieces.push(Piece::Break(index));
            continue;
        }
        if parts[index].chars().all(char::is_whitespace) {
            flush(&mut word, word_at, &mut pieces);
            index += 1;
            continue;
        }
        if word.is_empty() {
            word_at = index;
        }
        word.push_str(parts[index]);
        index += 1;
    }
    flush(&mut word, word_at, &mut pieces);

    let mut lines: Vec<(String, usize)> = Vec::new();
    let mut line = String::new();
    let mut start = 0usize;

    for piece in pieces {
        match piece {
            Piece::Break(next) => {
                lines.push((std::mem::take(&mut line), start));
                start = next;
            }
            Piece::Word { at, text } => {
                if line.is_empty() {
                    start = at;
                    line = text;
                } else if util::width(&line) + 1 + util::width(&text) <= width {
                    line.push(' ');
                    line.push_str(&text);
                } else {
                    lines.push((std::mem::take(&mut line), start));
                    start = at;
                    line = text;
                }
                // A word wider than the whole field is broken mid-word; there
                // is nowhere else to break it.
                while util::width(&line) > width {
                    let mut head = String::new();
                    let mut taken = 0usize;
                    for part in util::clusters(&line) {
                        if util::width(&head) + util::width(part) > width {
                            break;
                        }
                        head.push_str(part);
                        taken += 1;
                    }
                    line = util::clusters(&line).skip(taken).collect();
                    lines.push((head, start));
                    start += taken;
                }
            }
        }
    }
    lines.push((line, start));
    lines
}

/// Row, and screen column, of cluster `cursor` within laid-out `lines`.
fn cursor_at(lines: &[(String, usize)], cursor: usize) -> (usize, usize) {
    for (row, (text, start)) in lines.iter().enumerate().rev() {
        if cursor >= *start {
            // The column is how wide the line is up to the cursor, which is
            // not how many clusters that is.
            let into = (cursor - start).min(util::cluster_count(text));
            let head: String = util::clusters(text).take(into).collect();
            return (row, util::width(&head));
        }
    }
    (0, 0)
}

fn render_editor(frame: &mut Frame, app: &App, area: Rect) {
    let Some(editor) = app.editor.as_ref() else {
        return;
    };

    const LABEL_WIDTH: u16 = 12;
    const POPUP_WIDTH: u16 = 96;

    // Lay the text out first: how tall the dialog wants to be depends on how
    // much of it there is. Borders take 2 columns, the padding another 2.
    let width = POPUP_WIDTH.min(area.width).saturating_sub(4 + LABEL_WIDTH) as usize;
    let wrapped: Vec<Vec<(String, usize)>> = editor
        .fields
        .iter()
        .map(|field| wrap_field(field.value(), width))
        .collect();

    let mut needs = [1u16; 6];
    for (need, lines) in needs.iter_mut().zip(wrapped.iter()) {
        *need = lines.len() as u16;
    }

    // Borders, the top padding and the hint line with its own blank row above
    // are not the fields'.
    let chrome = 5;
    let heights = field_heights(area.height.saturating_sub(chrome), needs);
    let popup = centered_fixed(
        POPUP_WIDTH,
        heights.iter().sum::<u16>() + FIELDS.len() as u16 - 1 + chrome,
        area,
    );
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

    let [fields_area, _gap, hint_area] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(inner);

    // Each field, then a blank row between them.
    let mut constraints: Vec<Constraint> = Vec::new();
    for (i, height) in heights.iter().enumerate() {
        if i > 0 {
            constraints.push(Constraint::Length(1));
        }
        constraints.push(Constraint::Length(*height));
    }
    let slots = Layout::vertical(constraints).split(fields_area);

    for (i, name) in FIELDS.iter().enumerate() {
        let slot = slots[i * 2];
        if slot.height == 0 {
            continue;
        }
        let focused = i == editor.field;
        let field = &editor.fields[i];

        let lines = &wrapped[i];
        let (cursor_row, cursor_column) = cursor_at(lines, field.cursor());

        // Keep the cursor's line in view when a field holds more than it shows.
        let height = slot.height as usize;
        let first = if focused {
            cursor_row.saturating_sub(height - 1)
        } else {
            0
        };
        let shown = &lines[first.min(lines.len() - 1)..];

        for (row, (text, _)) in shown.iter().take(height).enumerate() {
            let label = if row > 0 {
                // The label belongs to the field, not to every line of it.
                " ".repeat(LABEL_WIDTH as usize)
            } else if focused {
                format!("▌{name:>9}  ")
            } else {
                format!("{name:>10}  ")
            };
            let label_style = if focused {
                Style::default().fg(ACCENT).bold()
            } else {
                Style::default().fg(DIM)
            };
            let value_style = if focused {
                Style::default().fg(BRIGHT)
            } else {
                Style::default().fg(TEXT)
            };

            let mut spans = vec![
                Span::styled(label, label_style),
                Span::styled(text.clone(), value_style),
            ];
            // Say when the field holds more than it is showing.
            let last = row + 1 == height.min(shown.len());
            if last && shown.len() > height {
                spans.push(Span::styled(" …", Style::default().fg(DIM)));
            }
            frame.render_widget(
                Paragraph::new(Line::from(spans)),
                Rect {
                    y: slot.y + row as u16,
                    height: 1,
                    ..slot
                },
            );
        }

        if focused {
            let column = slot.x + LABEL_WIDTH + cursor_column as u16;
            let row = slot.y + (cursor_row - first) as u16;
            frame.set_cursor_position(Position::new(
                column.min(slot.right().saturating_sub(1)),
                row.min(slot.bottom().saturating_sub(1)),
            ));
        }
    }

    let hint = Line::from(Span::styled(
        "Tab/j/k field   i insert   Esc normal   ZZ save   q cancel   \\n = new line",
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
        .border_style(Style::default().fg(MARK))
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
  5j             down five                 5G / 5gg    jump to line 5
  Ctrl-d/u       half page                 :42         jump to line 42
  h / l          back / forward            Enter       open
  q / Esc        back one screen           ZZ          save and quit
  ?              this help                 S           statistics

  The deck list carries a STATUS column: review needed, new cards, up to
  date, or empty. The same badge sits in the top-right of every screen.

DECKS
  s              study the selected deck   a           add a card
  Enter / l      browse its cards          dd          delete deck (confirms)

BROWSING
  /text          filter cards              n / N       next / previous match
  :noh           clear filter and marks    i / Enter   edit the card
  a / o          add a card                dd          delete card
  u              undo a delete             x           suspend / unsuspend
  m              cycle the STATUS column   :marked     show only flagged

  The selected word is shown in full in the `card` pane below the list, where
  the terminal is tall enough. `…` in its border means there is more than
  fits; Enter opens the whole thing in the editor.

REVIEWING
  space          reveal the answer         space       again: grade Good
  y              I know this               n           I don't know
  1 2 3 4        Again Hard Good Easy      u           undo the last answer
  m              cycle this word's status  e           edit this card
  x              suspend it                S           bury until later
  j k            scroll a long answer      Ctrl-d/u    half a card at a time
  q              end the session

  The bar counts cards seen off, not answers given: a card you answer Again
  shows up as `retrying` until you get it right.

WORD STATUS
  Every word carries a status of your own, shown in the browser's STATUS
  column and beside the term while reviewing. It is yours to set — unlike
  STATE (new / learn / rev), which only the scheduler moves.

  m              step to the next status   Ctrl-m      step back
  :status NAME   set it outright           :status none    clear it

      —          no status                 ◆ review    worth another look
      ★ marked   in the study pile         ▲ test      ready to test yourself

  m cycles  —  ->  review  ->  marked  ->  test  ->  —  , so a status can
  always be taken off again. Answering \"I don't know\" sets ★ marked for you,
  but only on a word you have not labelled yourself.

  M              drill every flagged word  :study marked   drill one status
  :marked        browse flagged only       :marked test    browse one status
  :noh           show everything again

  A status drill ignores due dates — it pulls those words forward on purpose.
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
  \\n             a new line in the Example and Notes fields

  Fields take as many lines as their text needs, and the dialog is only as
  tall as the card it holds. `…` means a field holds more than it is showing;
  it scrolls to follow the cursor as you type.

COMMANDS
  Tab completes commands and their arguments — statuses, study scopes,
  `:set` keys and values, and file paths for `:import` and `:export`. The
  first press fills in as far as the choices agree; after that Tab steps
  through them and Shift-Tab steps back.

  :w             save the collection       :q          back / quit
  :wq  :x        save and quit             :qa         save and quit outright
  :q!  :qa!      quit, asking first        ZQ          quit, no questions asked
  :deck NAME     create a deck             :rename N   rename this deck
  :import FILE [DECK]                      import a CSV or TSV
  :export FILE                             export this deck as CSV
  :set new=20    new cards per day         :forget     reset this deck's schedule
  :set grading=binary|sm2                  :status NAME    set a word's status
  :study marked  drill one status          :marked NAME    browse one status
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
                    Span::styled(keys.to_string(), Style::default().fg(MARK)),
                    Span::styled(rest.to_string(), Style::default().fg(TEXT)),
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
        let len = util::width(word);
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

/// The slice of `text` that keeps `cursor` — a cluster index — visible in
/// `width` columns. Returns the visible text and the cursor's column in it.
fn window(text: &str, cursor: usize, width: usize) -> (String, usize) {
    if width == 0 {
        return (String::new(), 0);
    }
    let parts: Vec<&str> = util::clusters(text).collect();
    let cursor = cursor.min(parts.len());

    // Walk back from the cursor while the columns last, keeping one for the
    // cursor itself. A double-width character fits whole or not at all.
    let mut start = cursor;
    let mut before = 0usize;
    while start > 0 {
        let next = util::width(parts[start - 1]);
        if before + next > width.saturating_sub(1) {
            break;
        }
        before += next;
        start -= 1;
    }

    let mut visible = String::new();
    let mut used = 0usize;
    for part in &parts[start..] {
        let next = util::width(part);
        if used + next > width {
            break;
        }
        visible.push_str(part);
        used += next;
    }
    (visible, before)
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
        assert_eq!(text, vec!["one", "two"], "the editor's \\n breaks the line");

        let lines = wrap_field("antidisestablishmentarianism", 10);
        assert_eq!(lines.len(), 3, "a long word is broken rather than hidden");
        let rebuilt: String = lines.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(rebuilt, "antidisestablishmentarianism");
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

        // A break the editor made itself puts the cursor on the new line.
        let lines = wrap_field("one\\ntwo", 40);
        assert_eq!(cursor_at(&lines, 5), (1, 0), "the n of \\n is not text");
    }

    #[test]
    fn fields_are_sized_to_what_they_hold() {
        // Room to spare: every field gets what it asks for, prose capped.
        let heights = field_heights(40, [1, 1, 1, 2, 6, 20]);
        assert_eq!(heights, [1, 1, 1, 2, 6, 8], "Notes capped at eight rows");

        // An empty card still gets a row per field to type into.
        assert_eq!(field_heights(40, [0, 0, 0, 0, 0, 0]), [1; 6]);

        // Tight: the greediest field gives rows up until the form fits.
        let heights = field_heights(12, [1, 1, 1, 3, 8, 8]);
        assert_eq!(
            heights.iter().sum::<u16>() + 5,
            12,
            "fields plus the gaps between them fill the height exactly"
        );
        assert!(
            heights.iter().all(|h| *h >= 1),
            "no field is squeezed out: {heights:?}"
        );

        // Nothing left to give: every field keeps its single row and the
        // dialog is simply taller than the terminal, which clamps it.
        assert_eq!(field_heights(1, [4, 4, 4, 4, 4, 4]), [1; 6]);
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
}
