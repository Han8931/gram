//! The card editor overlay, and the confirm prompt drawn the same way.

use super::*;

/// Rows each field gets: what its text needs, within reason, shrinking the
/// greediest field first when the form will not fit. Term, Pron and Pos never
/// need more than a line; Example and Notes get room to be read instead of
/// trailing off the edge, and Confusing a few lines for its look-alikes.
pub(super) fn field_heights(available: u16, needs: [u16; FIELDS.len()]) -> [u16; FIELDS.len()] {
    const CEILING: [u16; FIELDS.len()] = [1, 1, 1, 3, 8, 8, 3];
    // One blank row between fields.
    let gaps = FIELDS.len() as u16 - 1;

    let mut heights = [1u16; FIELDS.len()];
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
pub(super) fn wrap_field(value: &str, width: usize) -> Vec<(String, usize)> {
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
pub(super) fn cursor_at(lines: &[(String, usize)], cursor: usize) -> (usize, usize) {
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

pub(super) fn render_editor(frame: &mut Frame, app: &App, area: Rect) {
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

    let mut needs = [1u16; FIELDS.len()];
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

pub(super) fn render_confirm(frame: &mut Frame, app: &App, area: Rect) {
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
