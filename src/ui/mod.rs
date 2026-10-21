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

mod browse;
mod card;
mod decks;
mod editor;
mod help;
mod review;
mod stats;
#[cfg(test)]
mod tests;

use browse::*;
use card::*;
use decks::*;
use editor::*;
use help::*;
use review::*;
use stats::*;

// One palette, used by role rather than by name, so the screens stay
// consistent with each other. Fixed entries of the 256-colour table rather
// than the sixteen named colours: those are whatever the terminal's scheme
// says, and in plenty of schemes "dark grey" is all but invisible on black.
/// Chrome: titles, focus, the program's own voice.
const ACCENT: Color = Color::Indexed(111);
/// Secondary text: labels, captions, anything you read second. Still meant
/// to be read, so it stays well clear of the background.
const DIM: Color = Color::Indexed(245);
/// Lines and borders: structure you should see without reading.
const FAINT: Color = Color::Indexed(240);
/// Body text. Brighter than DIM, calmer than white.
const TEXT: Color = Color::Indexed(252);
/// Emphasis inside body text — a heading, a term, the answer.
const BRIGHT: Color = Color::Indexed(255);
/// Card states, shared by every count, badge and column that names them.
const NEW: Color = Color::Indexed(75);
const LEARN: Color = Color::Indexed(176);
const DUE: Color = Color::Indexed(114);
/// The ★ study pile, and emphasised words inside example sentences.
const MARK: Color = Color::Indexed(221);
/// Something needs attention: overdue cards, "I don't know", errors.
const WARN: Color = Color::Indexed(210);
/// Text drawn on a coloured badge.
const ON_BADGE: Color = Color::Indexed(234);
/// Selected row, and the header strip above a table. `drop_colour` tells a
/// badge from a table header by comparing against HEAD_BG, so a header strip
/// drawn in any other background would turn into reverse video under NO_COLOR.
const SEL_BG: Color = Color::Indexed(237);
const HEAD_BG: Color = Color::Indexed(235);

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

/// Whether to paint at all. <https://no-color.org>: set and not empty means
/// no colour, whatever its value.
fn colour_wanted() -> bool {
    static WANTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *WANTED.get_or_init(|| std::env::var_os("NO_COLOR").is_none_or(|value| value.is_empty()))
}

/// Strip the colour out of a drawn frame. Done on the finished buffer, so no
/// screen has to know about it. What colour alone was saying — this row is
/// selected, this is a badge — is said with reverse video instead.
fn drop_colour(frame: &mut Frame) {
    for cell in frame.buffer_mut().content.iter_mut() {
        let badge = !matches!(cell.bg, Color::Reset) && cell.bg != HEAD_BG;
        cell.fg = Color::Reset;
        cell.bg = Color::Reset;
        if badge {
            cell.modifier.insert(Modifier::REVERSED);
        }
    }
}

pub fn render(frame: &mut Frame, app: &mut App) {
    draw(frame, app);
    if !colour_wanted() {
        drop_colour(frame);
    }
}

fn draw(frame: &mut Frame, app: &mut App) {
    let [header, body, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    render_header(frame, app, header);

    // What `/` has found, worked out once a frame: the list picks the matches
    // out and the command line counts them, and both want the same answer.
    // Once the cursor moves off the match it was taken to, there is nothing
    // left to pick out.
    let found = if app.screen == Screen::Browse && app.search_highlighted() {
        app.search_matches()
    } else {
        Vec::new()
    };

    match app.screen {
        Screen::Decks => render_decks(frame, app, body),
        Screen::Browse => render_browse(frame, app, body, &found),
        Screen::Review => render_review(frame, app, body),
        Screen::Stats => render_stats(frame, app, body),
        Screen::Help => render_help(frame, app, body),
        Screen::Card => render_card_view(frame, app, body),
    }

    if app.editor.is_some() {
        render_editor(frame, app, body);
    }
    if app.confirm.is_some() {
        render_confirm(frame, app, body);
    }

    render_status(frame, app, status, &found);

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
    let labels: Vec<String> = completion
        .candidates
        .iter()
        .map(|candidate| format!(" {} ", menu_label(candidate)))
        .collect();
    let widths: Vec<usize> = labels.iter().map(|label| util::width(label)).collect();
    // Tab can walk past the right edge of the strip. Scroll with it, or the
    // candidate you just picked is the one you cannot see.
    let shown = wildmenu_window(&widths, completion.index.unwrap_or(0), area.width as usize);

    let mut spans = Vec::new();
    let marker = Style::default().fg(ACCENT);
    if shown.start > 0 {
        spans.push(Span::styled("‹", marker));
    }
    for index in shown.clone() {
        let style = if completion.index == Some(index) {
            Style::default().fg(ON_BADGE).bg(ACCENT).bold()
        } else {
            Style::default().fg(TEXT)
        };
        spans.push(Span::styled(labels[index].clone(), style));
    }
    if shown.end < labels.len() {
        spans.push(Span::styled("›", marker));
    }

    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(HEAD_BG)),
        area,
    );
}

/// Which candidates the strip has room for, with `selected` always among them.
/// Two columns are held back for the `‹ ›` markers whenever the list is wider
/// than the strip, so the window does not jitter by a column as it scrolls.
fn wildmenu_window(widths: &[usize], selected: usize, width: usize) -> std::ops::Range<usize> {
    if widths.iter().sum::<usize>() <= width {
        return 0..widths.len();
    }
    let budget = width.saturating_sub(2);
    let mut start = 0;
    loop {
        let mut end = start;
        let mut used = 0;
        while end < widths.len() && used + widths[end] <= budget {
            used += widths[end];
            end += 1;
        }
        // A candidate too wide for the whole strip still gets shown, clipped,
        // rather than scrolling for ever in search of a fit.
        if end > selected || start >= selected {
            return start..end.max(start + 1).min(widths.len());
        }
        start += 1;
    }
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
        Screen::Card => "word",
    };

    // Say when a session is not the plain due one, so a short queue of new
    // words is not mistaken for the whole of today's work.
    let drilling = app.session.as_ref().and_then(|s| match s.scope {
        Scope::Flagged(_) => Some(format!("  drilling {}", s.scope.label())),
        Scope::New(_) => Some("  learning new words".to_string()),
        Scope::Due | Scope::More(_) => None,
    });

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
        Span::styled(" gram ", Style::default().fg(ON_BADGE).bg(ACCENT).bold()),
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

fn render_status(frame: &mut Frame, app: &App, area: Rect, found: &[usize]) {
    // In `:`, `/` and `f` the status line becomes the command line, and the
    // search and filter say how many they have found as you type.
    if matches!(app.mode, Mode::Command | Mode::Search | Mode::Filter) {
        let (prefix, found) = match app.mode {
            Mode::Command => (":", None),
            Mode::Search => ("/", Some(found.len())),
            _ => ("filter ", Some(app.rows.len())),
        };
        let tally = match found {
            _ if app.line.is_empty() => String::new(),
            Some(0) => " no match ".to_string(),
            Some(1) => " 1 match ".to_string(),
            Some(n) => format!(" {n} matches "),
            None => String::new(),
        };
        let tally_style = if found == Some(0) {
            Style::default().fg(WARN)
        } else {
            Style::default().fg(DIM)
        };
        let reserved = util::width(prefix) + util::width(&tally) + 1;
        let width = (area.width as usize).saturating_sub(reserved);
        let (visible, offset) = window(app.line.value(), app.line.cursor(), width);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(prefix, Style::default().fg(ACCENT).bold()),
                Span::styled(visible, Style::default().fg(BRIGHT)),
            ])),
            area,
        );
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(tally, tally_style)).right_aligned()),
            area,
        );
        let column = area.x + util::width(prefix) as u16 + offset as u16;
        frame.set_cursor_position(Position::new(
            column.min(area.right().saturating_sub(1)),
            area.y,
        ));
        return;
    }

    let mode_style = match app.mode {
        Mode::Insert => Style::default().fg(ON_BADGE).bg(DUE).bold(),
        _ => Style::default().fg(ON_BADGE).bg(DIM).bold(),
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
        .border_style(Style::default().fg(FAINT))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(ACCENT).bold(),
        ))
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
