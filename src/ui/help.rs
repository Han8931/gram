//! The help screen.

use super::*;

pub(super) const HELP: &str = "\
MOVING AROUND
  j / k          down / up                 gg / G      first / last
  5j             down five                 5G / 5gg    jump to line 5
  Ctrl-d/u       half page                 :42         jump to line 42
  { / }          up / down ten rows        3}          down thirty
  h / l          back / forward            Enter       open
  Ctrl-c         says how to quit          :q          back / quit
  q / Esc        back one screen           ZZ          save and quit
  ?              this help                 S           statistics

  The deck list carries a STATUS column: review needed, new cards, up to
  date, or empty. The same badge sits in the top-right of every screen.

DECKS
  s              study the selected deck   a           add a card
  n              learn new words only      30n         thirty of them
  Enter / l      browse its cards          dd          delete deck (confirms)

  s studies what the schedule asks for: everything due today, with the day's
  ration of unseen words mixed in. n skips the backlog and teaches new words
  only — as many as `new_per_day`, or the count you type before it. Those
  words still enter the schedule, so they come back for review later.

BROWSING
  /text          jump to a word            n / N       next / previous match
  f text         filter the list           :noh        clear search and filter
  Enter          read the word in full     i           edit the card
  a / o          add a card                dd          delete card
  u / Ctrl-r     undo / redo a change      x           suspend / unsuspend
  m              cycle the STATUS column   :marked     show only flagged
  z              hide / show meanings      space       check the one word
  p              say the word out loud     :set voice=NAME     pick its voice

  The selected word is shown in the `card` pane below the list, where the
  terminal is tall enough. `…` in its border means there is more than fits.

  / and f both answer as you type, and Esc puts the cursor and the list back.
  Spelling need not be exact: case, accents and tone marks are ignored, and
  when no word matches as typed, ones a typo or two away are offered instead.
  The matches stay picked out until you move off them or press Esc; n / N
  bring them back.

READING A WORD
  Enter on a word opens it on its own, laid out like a revealed review card.
  j / k          scroll a long card        gg / G      top / bottom
  J / K          next / previous word      Tab         next word too
  i / e          edit it                   m / x       status / suspend
  q / h          back to the list          n / N       next / previous match
  p              say the word out loud

PRACTISING FROM THE LIST
  z covers every meaning in the browser — the DEFINITION column and the card
  pane both — leaving the words, their pronunciation and their part of speech.
  Press it again and they all come back. The border says `hidden` while it is
  on, and nothing is written to your collection: it is a way of looking at the
  list, not a setting.

  Work down the list recalling each meaning, and press space to uncover just
  the word under the cursor when you want to check yourself. Moving off it
  covers it again, so the row below is never already answered.

  z              hide every meaning        z again     show them all
  space          uncover this word         j / k       on to the next

  Nothing here grades anything or touches a due date. For answers that count
  towards the schedule, study the deck with s or drill a status with M.

REVIEWING
  space          reveal the answer         space       again: grade Good
  y              I know this               n           I don't know
  1 2 3 4        Again Hard Good Easy      u           undo the last answer
  m              cycle this word's status  e           edit this card
  x              suspend it                S           bury until later
  j k            scroll a long answer      Ctrl-d/u    half a card at a time
  p              say the word out loud     (after the reveal, on a reverse card)
  q              end the session

  The bar counts cards seen off, not answers given: a card you answer Again
  shows up as `retrying` until you get it right.

UNDO
  u              undo the last change      Ctrl-r      redo it
  3u             undo three changes        3Ctrl-r     redo three

  u takes back whatever you last did to the collection, not just a delete:
  edits, statuses, suspends, a reset deck, a deleted deck, a rename, an
  import, a setting. Press it again to keep walking back, up to 64 changes.
  Each one says what it was — `undid deleting Abash` — and takes you back to
  the screen and the row it happened on.

  Ctrl-r walks forward again. Making a change after undoing closes off the
  redo, as in Vim: what you undid stops being reachable once you have done
  something else instead.

  The history is this session's only. It is not written to your collection, so
  quitting ends it — a change you have saved and quit on is not undoable, and
  what protects you there is `:restore`.

  While reviewing, u means the finer thing it always did: take back my last
  answer, queue and all. The change history covers everything else — and
  never answers: undoing an edit made before a session leaves the session's
  answers exactly as they were.

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
  `:set` keys and values, and file paths for `:import`, `:export` and
  `:restore`. The first press fills in as far as the choices agree; after
  that Tab steps through them and Shift-Tab steps back.

  :w             save the collection       :q          back / quit
  :wq  :x        save and quit             :qa         save and quit outright
  :q!  :qa!      quit, asking first        ZQ          quit, no questions asked
  :deck NAME     create a deck             :rename N   rename this deck
  :import FILE [DECK]                      import a CSV or TSV
  :export FILE                             export this deck as CSV
  :set new=20    new cards per day         :forget     reset this deck's schedule
  :set grading=binary|sm2                  :status NAME    set a word's status
  :set reverse=on|off                      ask for the word from its meaning
  :set voice=NAME                          the voice p speaks this deck in
  :study marked  drill one status          :marked NAME    browse one status
  :study more [N]                          N extra new cards, today only
  :study new [N]                           N new words, no reviews
  :restore       list kept copies          :restore N      put one back
  :study :browse :marked :stats :help

SAVING AND RECOVERY
  gram writes through a temp file and renames, so an interrupted save cannot
  truncate the collection. If something else has written to the collection
  since this session loaded it, that version is copied aside as
  `collection.conflict-<date>-<time>.json` before yours replaces it; if the
  copy cannot be made, the save is refused rather than losing their work.
  `:restore` lists those copies by their stamp, newest first, and
  `:restore 1` puts one back — keeping what it displaces the same way, so a
  wrong restore is itself undoable. It ends any session in progress.

SCHEDULING
  gram uses SM-2. Again resets the card and shows it again this session;
  Hard, Good and Easy push it further out, scaled by the card's ease factor.
  Each deck introduces at most `new_per_day` unseen cards per day;
  :study more goes past that for today without changing the limit, and
  n / :study new teaches unseen words on their own, backlog and ration aside.
  If the reviews are piling up faster than you want to meet them, raise the
  ration with :set new=N — the words you learn today are what you review
  tomorrow.

CSV FORMAT
  Recognised headers, in any order:
      term, pron, pos, definition, example, notes, confusing
  Without a header row, columns are read as term, definition, example.
  Re-importing a file updates existing terms instead of duplicating them,
  so your scheduling survives a corrected export.
";

pub(super) fn render_help(frame: &mut Frame, app: &mut App, area: Rect) {
    let lines: Vec<Line> = HELP
        .lines()
        .map(|line| {
            if !line.starts_with(' ') && !line.is_empty() {
                Line::from(Span::styled(
                    line.to_string(),
                    Style::default().fg(MARK).bold(),
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
                    Span::styled(keys.to_string(), Style::default().fg(ACCENT)),
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
