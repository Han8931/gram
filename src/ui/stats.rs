//! The statistics screen.

use super::*;

/// A rate as a whole percentage, or an em dash when there is nothing to rate.
pub(super) fn percent(rate: Option<f32>) -> String {
    match rate {
        Some(rate) => format!("{:.0}%", rate * 100.0),
        None => "—".to_string(),
    }
}

pub(super) fn render_stats(frame: &mut Frame, app: &mut App, area: Rect) {
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

pub(super) fn bar_rows(data: &[(chrono::NaiveDate, u64)], color: Color) -> Vec<Line<'static>> {
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
