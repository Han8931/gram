use super::*;
use crate::model::Card;

fn key(ch: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)
}

/// Id of the one deck `app_with_cards` builds. Ids start at 1, so `0`
/// stays available as "no deck".
const DECK: u64 = 1;

/// Draw one frame into a throwaway terminal. The window the list ends up
/// showing lives in the render, so testing it means rendering.
fn draw(app: &mut App, width: u16, height: u16) {
    let backend = ratatui::backend::TestBackend::new(width, height);
    let mut terminal = ratatui::Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| crate::ui::render(frame, app))
        .expect("draw");
}

fn app_with_cards(count: usize) -> App {
    let mut db = Db::default();
    let index = db.add_deck("test".into());
    let today = Local::now().date_naive();
    for i in 0..count {
        let id = db.next_card_id();
        db.decks[index].cards.push(Card::new(
            id,
            format!("term{i}"),
            format!("definition {i}"),
            today,
        ));
    }
    assert_eq!(
        db.decks[index].id, DECK,
        "DECK is out of step with add_deck"
    );
    let path =
        std::env::temp_dir().join(format!("gram-test-{}-{}.json", std::process::id(), count));
    let mut app = App::new(db, path);
    app.refresh_rows();
    app
}

#[test]
fn escape_round_trips_newlines_and_backslashes() {
    for original in ["a\nb", "back\\slash", "mix\\n\nreal", "", "trailing\\"] {
        assert_eq!(unescape(&escape(original)), original, "{original:?}");
    }
}

#[test]
fn queue_contains_every_available_card() {
    let mut app = app_with_cards(5);
    app.db.decks[0].new_per_day = 100;
    let queue = app.build_queue(DECK, Scope::Due);
    assert_eq!(queue.len(), 5);
}

#[test]
fn queue_respects_the_daily_new_limit() {
    let mut app = app_with_cards(50);
    app.db.decks[0].new_per_day = 7;
    assert_eq!(app.build_queue(DECK, Scope::Due).len(), 7);
}

#[test]
fn suspended_cards_stay_out_of_the_queue() {
    let mut app = app_with_cards(4);
    app.db.decks[0].new_per_day = 100;
    app.db.decks[0].cards[1].suspended = true;
    assert_eq!(app.build_queue(DECK, Scope::Due).len(), 3);
}

#[test]
fn study_more_adds_new_cards_past_the_days_ration() {
    let mut app = app_with_cards(10);
    app.db.decks[0].new_per_day = 3;
    app.db.decks[0].spend_new(app.today, 3);
    assert!(app.build_queue(DECK, Scope::Due).is_empty());
    assert_eq!(app.build_queue(DECK, Scope::More(4)).len(), 4);
    assert_eq!(app.build_queue(DECK, Scope::More(50)).len(), 10);
}

#[test]
fn study_more_leaves_the_daily_limit_alone() {
    let mut app = app_with_cards(10);
    app.db.decks[0].new_per_day = 2;
    app.run_command("study more 5");
    assert_eq!(app.session.as_ref().unwrap().queue.len(), 7);
    assert_eq!(app.db.decks[0].new_per_day, 2);
}

#[test]
fn study_more_defaults_to_another_days_ration() {
    let mut app = app_with_cards(30);
    app.db.decks[0].new_per_day = 12;
    app.db.decks[0].spend_new(app.today, 12);
    app.run_command("study more");
    assert_eq!(app.session.as_ref().unwrap().queue.len(), 12);
}

#[test]
fn only_study_more_takes_a_number() {
    let mut app = app_with_cards(3);
    app.run_command("study more zero");
    assert!(app.session.is_none());
    app.run_command("study marked 5");
    assert!(app.session.is_none());
}

/// Turn the first `count` cards into reviews that fell due today.
fn make_due_reviews(app: &mut App, count: usize) {
    let today = app.today;
    for card in app.db.decks[0].cards.iter_mut().take(count) {
        card.state = CardState::Review;
        card.due = today;
        card.interval = 1;
    }
}

#[test]
fn study_new_leaves_the_review_backlog_alone() {
    let mut app = app_with_cards(10);
    make_due_reviews(&mut app, 6);
    assert_eq!(app.build_queue(DECK, Scope::Due).len(), 10, "6 due + 4 new");
    let queue = app.build_queue(DECK, Scope::New(20));
    assert_eq!(queue.len(), 4);
    for id in queue {
        let card = app.db.decks[0].card(id).expect("queued card");
        assert_eq!(card.state, CardState::New, "{} is not unseen", card.term);
    }
}

#[test]
fn study_new_is_not_capped_by_the_days_ration() {
    let mut app = app_with_cards(10);
    app.db.decks[0].new_per_day = 3;
    app.db.decks[0].spend_new(app.today, 3);
    assert!(app.build_queue(DECK, Scope::Due).is_empty());
    assert_eq!(app.build_queue(DECK, Scope::New(6)).len(), 6);
}

#[test]
fn study_new_takes_a_count_and_defaults_to_a_ration() {
    let mut app = app_with_cards(30);
    app.db.decks[0].new_per_day = 12;
    app.run_command("study new 5");
    assert_eq!(app.session.as_ref().unwrap().queue.len(), 5);
    app.run_command("study new");
    assert_eq!(app.session.as_ref().unwrap().queue.len(), 12);
    assert_eq!(app.db.decks[0].new_per_day, 12, "the limit is untouched");
}

#[test]
fn study_new_suspends_nothing_and_skips_suspended() {
    let mut app = app_with_cards(4);
    app.db.decks[0].cards[0].suspended = true;
    assert_eq!(app.build_queue(DECK, Scope::New(10)).len(), 3);
}

#[test]
fn n_on_the_deck_list_studies_new_words() {
    let mut app = app_with_cards(30);
    app.db.decks[0].new_per_day = 8;
    app.on_key(key('n')).expect("key");
    let session = app.session.as_ref().expect("session");
    assert_eq!(session.scope, Scope::New(8));
    assert_eq!(session.queue.len(), 8);
    // A count says how many: `15n`.
    app.on_key(key('q')).expect("key");
    app.on_key(key('1')).expect("key");
    app.on_key(key('5')).expect("key");
    app.on_key(key('n')).expect("key");
    assert_eq!(app.session.as_ref().expect("session").queue.len(), 15);
}

#[test]
fn new_words_answered_count_against_the_days_ration() {
    let mut app = app_with_cards(10);
    app.db.decks[0].new_per_day = 4;
    app.run_command("study new 2");
    for _ in 0..2 {
        app.reveal();
        app.answer(Grade::Good);
    }
    assert_eq!(app.db.decks[0].new_allowance(app.today), 2);
}

#[test]
fn answering_good_advances_the_queue_and_logs() {
    let mut app = app_with_cards(3);
    app.db.decks[0].new_per_day = 100;
    app.start_session(DECK, Scope::Due);
    assert_eq!(app.session.as_ref().unwrap().queue.len(), 3);

    app.answer(Grade::Good);
    assert_eq!(app.session.as_ref().unwrap().queue.len(), 2);
    assert_eq!(app.db.log.len(), 1);
    assert_eq!(app.db.decks[0].intro_count, 1);
}

#[test]
fn the_log_records_the_deck_id_not_its_index() {
    let mut app = app_with_cards(2);
    app.db.decks[0].new_per_day = 100;
    app.db.decks[0].id = 77;
    app.start_session(77, Scope::Due);
    app.answer(Grade::Good);
    assert_eq!(app.db.log[0].deck_id, 77);
}

#[test]
fn answering_again_puts_the_card_back() {
    let mut app = app_with_cards(12);
    app.db.decks[0].new_per_day = 100;
    app.start_session(DECK, Scope::Due);
    let first = app.session.as_ref().unwrap().current().unwrap();

    app.answer(Grade::Again);
    let queue = &app.session.as_ref().unwrap().queue;
    assert_eq!(queue.len(), 12, "card returns to the queue");
    assert!(queue.contains(&first));
    assert_ne!(queue.front(), Some(&first), "but not immediately");
}

/// Type a `:` command line, keystroke by keystroke.
fn typed(app: &mut App, text: &str) {
    app.on_key(key(':')).unwrap();
    for ch in text.chars() {
        app.on_key(key(ch)).unwrap();
    }
}

fn tab(app: &mut App) {
    app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .unwrap();
}

fn back_tab(app: &mut App) {
    app.on_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE))
        .unwrap();
}

#[test]
fn tab_completes_a_lone_command_outright() {
    let mut app = app_with_cards(1);
    typed(&mut app, "expo");
    tab(&mut app);
    assert_eq!(
        app.line.value(),
        "export ",
        "and spaces along to the argument"
    );
    assert!(app.completion.is_none(), "nothing left to choose");
}

#[test]
fn tab_fills_the_shared_prefix_then_cycles() {
    let mut app = app_with_cards(1);
    typed(&mut app, "st");

    // stats, status and study all start with st — nothing to fill in yet,
    // so Tab goes straight to offering them.
    tab(&mut app);
    assert_eq!(app.line.value(), "stats");
    let completion = app.completion.as_ref().expect("candidates are on offer");
    assert_eq!(completion.candidates, vec!["stats", "status", "study"]);
    assert_eq!(completion.index, Some(0));

    tab(&mut app);
    assert_eq!(app.line.value(), "status");
    tab(&mut app);
    assert_eq!(app.line.value(), "study");
    tab(&mut app);
    assert_eq!(app.line.value(), "stats", "and round again");

    back_tab(&mut app);
    assert_eq!(app.line.value(), "study", "backwards too");
}

#[test]
fn tab_fills_in_as_far_as_the_candidates_agree() {
    let mut app = app_with_cards(1);
    typed(&mut app, "q");
    tab(&mut app);
    // q, q!, qa, qa! and quit all start with q, so the first press has
    // nothing to add and offers the first instead.
    assert_eq!(app.line.value(), "q");
    assert_eq!(
        app.completion.as_ref().map(|c| c.candidates.clone()),
        Some(vec![
            "q".to_string(),
            "q!".to_string(),
            "qa".to_string(),
            "qa!".to_string(),
            "quit".to_string(),
        ])
    );

    let mut app = app_with_cards(1);
    typed(&mut app, "stud");
    tab(&mut app);
    assert_eq!(app.line.value(), "study ", "one match, taken whole");
}

#[test]
fn tab_completes_arguments_not_just_commands() {
    let mut app = app_with_cards(1);
    typed(&mut app, "status re");
    tab(&mut app);
    assert_eq!(app.line.value(), "status review ");

    let mut app = app_with_cards(1);
    typed(&mut app, "set grading=b");
    tab(&mut app);
    assert_eq!(app.line.value(), "set grading=binary ");
}

#[test]
fn an_unfinished_word_does_not_get_a_space() {
    let mut app = app_with_cards(1);
    typed(&mut app, "set g");
    tab(&mut app);
    assert_eq!(
        app.line.value(),
        "set grading=",
        "the value still has to be typed, so no space lands in between"
    );

    // And a second Tab carries on into the values.
    tab(&mut app);
    assert_eq!(app.line.value(), "set grading=binary");
}

#[test]
fn typing_ends_the_completion() {
    let mut app = app_with_cards(1);
    typed(&mut app, "st");
    tab(&mut app);
    assert!(app.completion.is_some());

    app.on_key(key('a')).unwrap();
    assert!(app.completion.is_none(), "the menu goes away");
    assert_eq!(app.line.value(), "statsa", "and the key is just typed");
}

#[test]
fn a_completed_command_runs() {
    let mut app = app_with_cards(1);
    typed(&mut app, "brow");
    tab(&mut app);
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.screen, Screen::Browse);
    assert!(app.completion.is_none());
}

#[test]
fn tab_does_nothing_for_an_unknown_command() {
    let mut app = app_with_cards(1);
    typed(&mut app, "zzz");
    tab(&mut app);
    assert_eq!(app.line.value(), "zzz");
    assert!(app.completion.is_none());
}

#[test]
fn search_is_not_completed() {
    let mut app = app_with_cards(1);
    app.enter_browse(DECK);
    app.on_key(key('/')).unwrap();
    for ch in "st".chars() {
        app.on_key(key(ch)).unwrap();
    }
    tab(&mut app);
    assert_eq!(app.line.value(), "st", "a search is free text");
    assert!(app.completion.is_none());
}

#[test]
fn saving_over_someone_elses_write_keeps_their_version() {
    let dir = std::env::temp_dir().join(format!("gram-conflict-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("collection.json");

    // This app loaded when there was no collection at this path yet.
    let mut app = app_with_cards(2);
    app.path = path.clone();
    app.written = store::modified(&path);
    assert_eq!(app.written, None, "nothing written there yet");

    // Something else writes one underneath us — `gram import` in another
    // terminal, say.
    let theirs = r#"{"decks":[{"id":9,"name":"theirs","cards":[]}]}"#;
    std::fs::write(&path, theirs).unwrap();

    let aside = app.save().unwrap().expect("the clash is noticed");
    assert_eq!(
        std::fs::read_to_string(&aside).unwrap(),
        theirs,
        "their collection is kept, not lost"
    );
    let ours = store::load_from(&path).unwrap();
    assert_eq!(ours.decks[0].name, "test", "and ours is what is loaded now");

    // Saving again is not a conflict: the last write was our own.
    assert!(app.save().unwrap().is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

/// Keeping their version is the price of overwriting it. If the copy
/// cannot be made, refusing to save loses nothing — the session is still
/// here to retry — whereas writing anyway loses their collection.
#[test]
#[cfg(unix)]
fn a_save_that_cannot_keep_their_version_refuses_to_overwrite() {
    use std::os::unix::fs::PermissionsExt;

    let dir = std::env::temp_dir().join(format!("gram-noconflict-{}", std::process::id()));
    let writable = |mode: u32| {
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(mode));
    };
    // An earlier failure can leave this directory unwritable, which would
    // make every later run fail for the wrong reason.
    writable(0o700);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("collection.json");

    let mut app = app_with_cards(2);
    app.path = path.clone();
    app.written = store::modified(&path);

    let theirs = r#"{"decks":[{"id":9,"name":"theirs","cards":[]}]}"#;
    std::fs::write(&path, theirs).unwrap();

    // A directory nothing can be created in is the simplest way to make
    // the backup fail. Root ignores the mode, so check before trusting it.
    writable(0o500);
    if std::fs::write(dir.join("probe"), "").is_ok() {
        writable(0o700);
        let _ = std::fs::remove_dir_all(&dir);
        return; // running as root; there is nothing to test
    }

    // Nothing may panic while the directory is locked, or the next run
    // inherits it — so the save happens here and the asserts come after.
    app.dirty = true;
    let outcome = app.save();
    writable(0o700);

    let err = outcome.expect_err("the save must not go through");
    assert!(
        format!("{err:#}").contains("could not be backed up"),
        "the error says why: {err:#}"
    );
    assert!(app.dirty, "and the session still knows it is unsaved");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        theirs,
        "their collection is untouched"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn restore_puts_a_kept_copy_back() {
    let dir = std::env::temp_dir().join(format!("gram-apprestore-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("collection.json");

    let mut app = app_with_cards(3);
    app.path = path.clone();
    app.written = None;

    // Something else writes a collection, so saving keeps theirs aside.
    let mut theirs = Db::default();
    let index = theirs.add_deck("theirs".into());
    theirs.decks[index].cards.push(Card::new(
        1,
        "abash".into(),
        "to embarrass".into(),
        app.today,
    ));
    store::save_to(&theirs, &path).unwrap();
    let aside = app.save().unwrap().expect("a conflict");

    // With a session open and ids in flight, to prove a restore clears it.
    app.start_session(DECK, Scope::Due);
    assert!(app.session.is_some());

    // `:restore` on its own only reports what there is.
    typed(&mut app, "restore");
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert!(app.confirm.is_none(), "listing asks nothing");
    assert!(
        app.status.text.contains(&store::backup_label(&aside)),
        "it names the copy: {:?}",
        app.status.text
    );
    assert_eq!(app.db.decks[0].name, "test", "and changes nothing");

    // `:restore 1` asks first, and n is a no.
    typed(&mut app, "restore 1");
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert!(app.confirm.is_some(), "a restore is confirmed");
    app.on_key(key('n')).unwrap();
    assert_eq!(app.db.decks[0].name, "test", "declined, nothing moved");

    typed(&mut app, "restore 1");
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    app.on_key(key('y')).unwrap();

    assert_eq!(app.db.decks.len(), 1);
    assert_eq!(app.db.decks[0].name, "theirs", "their deck is live now");
    assert_eq!(app.db.decks[0].cards[0].term, "abash");
    assert!(app.session.is_none(), "a session over other cards is ended");
    assert_eq!(app.screen, Screen::Decks);
    assert!(!app.dirty, "the restored collection is what is on disk");
    assert_eq!(
        store::load_from(&path).unwrap().decks[0].name,
        "theirs",
        "and it was written, not just loaded"
    );

    // Our own collection was kept on the way out, so this is reversible.
    let kept = store::conflict_copies(&path);
    assert!(
        kept.iter()
            .any(|p| store::load_from(p).is_ok_and(|db| db.decks.iter().any(|d| d.name == "test"))),
        "what the restore displaced is still there: {kept:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_long_session_checkpoints_as_it_goes() {
    let mut app = app_with_cards(30);
    app.path = std::env::temp_dir().join(format!("gram-checkpoint-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&app.path);
    app.db.decks[0].new_per_day = 100;
    app.start_session(DECK, Scope::Due);

    for _ in 0..CHECKPOINT_EVERY - 1 {
        app.answer(Grade::Good);
    }
    assert!(app.dirty, "nothing written yet");
    assert!(!app.path.exists());

    app.answer(Grade::Good);
    assert!(!app.dirty, "the tenth answer checkpoints");
    let saved = store::load_from(&app.path).expect("a readable collection");
    assert_eq!(
        saved.log.len(),
        CHECKPOINT_EVERY as usize,
        "every answer so far is on disk"
    );
    let _ = std::fs::remove_file(&app.path);
}

#[test]
fn retention_leaves_out_drills_retries_and_first_sightings() {
    let mut app = app_with_cards(4);
    app.db.decks[0].new_per_day = 100;

    // Learn all four, which is not retention: nothing is being remembered.
    app.start_session(DECK, Scope::Due);
    for _ in 0..4 {
        app.answer(Grade::Good);
    }
    assert_eq!(app.review_retention(), None, "only new cards so far");
    assert_eq!(app.answer_accuracy(), Some(1.0));

    // A day later they come round as scheduled reviews: one Again, then
    // the retry, then two more Goods.
    app.today += Duration::days(1);
    for card in &mut app.db.decks[0].cards {
        card.due = app.today;
    }
    app.start_session(DECK, Scope::Due);
    app.answer(Grade::Again);
    app.answer(Grade::Good);
    app.answer(Grade::Good);
    app.answer(Grade::Good);

    // Four cards, four first answers, one of them wrong. The retry on the
    // lapsed card does not get a second vote.
    assert_eq!(app.review_retention(), Some(0.75));

    // A drill the same day does not move retention, but does move accuracy.
    let before = app.review_retention();
    app.db.decks[0].cards[0].status = Study::Marked;
    app.start_session(DECK, Scope::Flagged(None));
    app.answer(Grade::Again);
    assert_eq!(app.review_retention(), before, "drills are not retention");
    assert!(
        app.answer_accuracy().unwrap() < 0.8,
        "but every answer counts towards accuracy"
    );
}

#[test]
fn an_older_log_counts_as_scheduled_reviews() {
    let mut app = app_with_cards(2);
    // Entries written before drills and new cards were recorded.
    for (card_id, grade) in [(1, Grade::Good.index()), (2, Grade::Again.index())] {
        app.db.log.push(ReviewLog {
            date: app.today,
            deck_id: DECK,
            card_id,
            grade,
            drill: false,
            learning: false,
        });
    }
    assert_eq!(app.review_retention(), Some(0.5));
}

#[test]
fn a_retry_does_not_count_as_progress() {
    let mut app = app_with_cards(3);
    app.db.decks[0].new_per_day = 100;
    app.start_session(DECK, Scope::Due);
    let first = app.session.as_ref().unwrap().current().unwrap();

    // Again, Good, Good: three answers, but only two cards seen off.
    app.answer(Grade::Again);
    app.answer(Grade::Good);
    app.answer(Grade::Good);

    let session = app.session.as_ref().unwrap();
    assert_eq!(session.answered, 3, "every answer is still counted");
    assert_eq!(session.completed, 2, "the retried card is not done yet");
    assert_eq!(session.retrying.len(), 1);
    assert!(session.retrying.contains(&first));
    assert!(
        session.completed < session.initial,
        "the bar cannot read 100% with a card still in the queue"
    );

    // Finishing the retry completes it and clears the retry count.
    while app.session.as_ref().is_some_and(|s| !s.queue.is_empty()) {
        app.answer(Grade::Good);
    }
    assert!(app.session.is_none(), "the session ended");
}

#[test]
fn undo_puts_the_retry_count_back() {
    let mut app = app_with_cards(3);
    app.db.decks[0].new_per_day = 100;
    app.start_session(DECK, Scope::Due);
    let first = app.session.as_ref().unwrap().current().unwrap();

    app.answer(Grade::Again);
    assert_eq!(app.session.as_ref().unwrap().retrying.len(), 1);
    app.undo_answer();
    let session = app.session.as_ref().unwrap();
    assert!(session.retrying.is_empty(), "the retry is undone too");
    assert_eq!(session.completed, 0);

    // And the other way round: undoing a completion un-completes it
    // without inventing a retry.
    app.answer(Grade::Good);
    assert_eq!(app.session.as_ref().unwrap().completed, 1);
    app.undo_answer();
    let session = app.session.as_ref().unwrap();
    assert_eq!(session.completed, 0);
    assert!(session.retrying.is_empty());
    assert_eq!(session.current(), Some(first), "and it is up again");
}

#[test]
fn a_card_retried_twice_is_counted_once() {
    let mut app = app_with_cards(3);
    app.db.decks[0].new_per_day = 100;
    app.start_session(DECK, Scope::Due);

    // Again on the first card, then Again on it again when it comes back.
    app.answer(Grade::Again);
    app.answer(Grade::Good);
    app.answer(Grade::Good);
    app.answer(Grade::Again);
    let session = app.session.as_ref().unwrap();
    assert_eq!(session.retrying.len(), 1, "one card is awaiting another go");
    assert_eq!(session.completed, 2);
}

#[test]
fn suspending_mid_session_counts_as_dealt_with() {
    let mut app = app_with_cards(3);
    app.db.decks[0].new_per_day = 100;
    app.start_session(DECK, Scope::Due);

    app.on_key(key('x')).unwrap();
    let session = app.session.as_ref().unwrap();
    assert_eq!(session.completed, 1, "it left the queue for good");
    assert_eq!(session.queue.len(), 2);
}

#[test]
fn j_and_k_scroll_the_card_rather_than_the_deck_list() {
    let mut app = app_with_cards(3);
    app.db.decks[0].new_per_day = 100;
    app.start_session(DECK, Scope::Due);
    let before = app.decks_sel;

    app.on_key(key('j')).unwrap();
    assert_eq!(app.card_scroll, 1);
    app.on_key(key('k')).unwrap();
    assert_eq!(app.card_scroll, 0);
    app.on_key(key('k')).unwrap();
    assert_eq!(app.card_scroll, 0, "and does not go negative");
    assert_eq!(app.decks_sel, before, "the deck cursor stays put");
}

#[test]
fn turning_a_card_over_starts_at_the_top() {
    let mut app = app_with_cards(3);
    app.db.decks[0].new_per_day = 100;
    app.start_session(DECK, Scope::Due);

    app.on_key(key(' ')).unwrap();
    app.on_key(key('j')).unwrap();
    app.on_key(key('j')).unwrap();
    assert_eq!(app.card_scroll, 2);

    app.answer(Grade::Good);
    assert_eq!(app.card_scroll, 0, "the next card starts at the top");

    app.on_key(key(' ')).unwrap();
    app.on_key(key('j')).unwrap();
    app.undo_answer();
    assert_eq!(app.card_scroll, 0, "and so does an undone one");
}

#[test]
fn undo_restores_scheduling_and_queue_position() {
    let mut app = app_with_cards(3);
    app.db.decks[0].new_per_day = 100;
    app.start_session(DECK, Scope::Due);
    let first = app.session.as_ref().unwrap().current().unwrap();
    let before = app.db.decks[0].card(first).unwrap().clone();

    app.answer(Grade::Good);
    app.undo_answer();

    let after = app.db.decks[0].card(first).unwrap();
    assert_eq!(after.state, before.state);
    assert_eq!(after.interval, before.interval);
    assert_eq!(app.session.as_ref().unwrap().current(), Some(first));
    assert_eq!(app.db.log.len(), 0);
    assert_eq!(app.db.decks[0].intro_count, 0);
}

#[test]
fn undo_after_again_removes_the_requeued_copy() {
    let mut app = app_with_cards(12);
    app.db.decks[0].new_per_day = 100;
    app.start_session(DECK, Scope::Due);
    let first = app.session.as_ref().unwrap().current().unwrap();

    app.answer(Grade::Again);
    app.undo_answer();

    let queue = &app.session.as_ref().unwrap().queue;
    assert_eq!(queue.len(), 12);
    assert_eq!(queue.iter().filter(|id| **id == first).count(), 1);
    assert_eq!(queue.front(), Some(&first));
}

#[test]
fn session_ends_when_the_queue_empties() {
    let mut app = app_with_cards(1);
    app.db.decks[0].new_per_day = 100;
    app.start_session(DECK, Scope::Due);
    app.answer(Grade::Good);
    assert!(app.session.is_none());
    assert_eq!(app.screen, Screen::Decks);
}

#[test]
fn binary_mode_marks_a_word_you_miss() {
    let mut app = app_with_cards(4);
    app.db.grading = Grading::Binary;
    app.db.decks[0].new_per_day = 100;
    app.start_session(DECK, Scope::Due);
    let id = app.session.as_ref().unwrap().current().unwrap();

    app.answer(Grade::Again);
    assert!(app.db.decks[0].card(id).unwrap().is_flagged());
}

#[test]
fn sm2_mode_leaves_marks_alone() {
    let mut app = app_with_cards(4);
    app.db.grading = Grading::Sm2;
    app.db.decks[0].new_per_day = 100;
    app.start_session(DECK, Scope::Due);
    let id = app.session.as_ref().unwrap().current().unwrap();

    app.answer(Grade::Again);
    assert!(!app.db.decks[0].card(id).unwrap().is_flagged());
}

#[test]
fn knowing_a_word_does_not_mark_it() {
    let mut app = app_with_cards(4);
    app.db.grading = Grading::Binary;
    app.db.decks[0].new_per_day = 100;
    app.start_session(DECK, Scope::Due);
    let id = app.session.as_ref().unwrap().current().unwrap();

    app.answer(Grade::Good);
    assert!(!app.db.decks[0].card(id).unwrap().is_flagged());
}

#[test]
fn undo_clears_a_mark_the_miss_added() {
    let mut app = app_with_cards(4);
    app.db.grading = Grading::Binary;
    app.db.decks[0].new_per_day = 100;
    app.start_session(DECK, Scope::Due);
    let id = app.session.as_ref().unwrap().current().unwrap();

    app.answer(Grade::Again);
    assert!(app.db.decks[0].card(id).unwrap().is_flagged());

    app.undo_answer();
    assert!(!app.db.decks[0].card(id).unwrap().is_flagged());
}

#[test]
fn a_marked_drill_ignores_due_dates() {
    let mut app = app_with_cards(6);
    // Everything is new and none of it is due, so a normal session with a
    // zero new-card ration would be empty.
    app.db.decks[0].new_per_day = 0;
    assert!(app.build_queue(DECK, Scope::Due).is_empty());

    app.db.decks[0].cards[1].status = Study::Marked;
    app.db.decks[0].cards[4].status = Study::Marked;
    let queue = app.build_queue(DECK, Scope::Flagged(None));
    assert_eq!(queue.len(), 2);
}

#[test]
fn a_marked_drill_skips_suspended_cards() {
    let mut app = app_with_cards(4);
    for card in &mut app.db.decks[0].cards {
        card.status = Study::Marked;
    }
    app.db.decks[0].cards[0].suspended = true;
    assert_eq!(app.build_queue(DECK, Scope::Flagged(None)).len(), 3);
}

#[test]
fn m_cycles_the_status_in_the_browser() {
    let mut app = app_with_cards(3);
    app.enter_browse(DECK);
    app.browse_sel = 2;

    let status = |app: &App| app.db.decks[0].cards[2].status;
    assert_eq!(status(&app), Study::None);

    for expected in [Study::Review, Study::Marked, Study::Test, Study::None] {
        app.on_key(key('m')).unwrap();
        assert_eq!(status(&app), expected, "m steps forward through the cycle");
    }
}

#[test]
fn z_hides_every_meaning_and_shows_them_again() {
    let mut app = app_with_cards(3);
    app.enter_browse(DECK);

    app.on_key(key('z')).unwrap();
    assert!(app.hide_meanings, "z hides");
    assert!((0..3).all(|row| app.meaning_hidden(row)), "the whole list");

    app.on_key(key('z')).unwrap();
    assert!(!app.hide_meanings, "z again shows");
    assert!((0..3).all(|row| !app.meaning_hidden(row)));
}

#[test]
fn space_uncovers_only_the_selected_word() {
    let mut app = app_with_cards(3);
    app.enter_browse(DECK);
    app.on_key(key('z')).unwrap();
    app.on_key(key(' ')).unwrap();

    assert!(!app.meaning_hidden(0), "the one under the cursor");
    assert!(app.meaning_hidden(1), "and no other");
    assert!(app.hide_meanings, "space does not leave the mode");
}

#[test]
fn moving_on_covers_the_word_space_uncovered() {
    let mut app = app_with_cards(3);
    app.enter_browse(DECK);
    app.on_key(key('z')).unwrap();
    app.on_key(key(' ')).unwrap();
    app.on_key(key('j')).unwrap();

    assert!(app.meaning_hidden(1), "the next word starts covered");
    assert!(app.meaning_hidden(0), "and the last one is covered again");
}

#[test]
fn space_does_nothing_while_the_meanings_are_showing() {
    let mut app = app_with_cards(3);
    app.enter_browse(DECK);
    app.on_key(key(' ')).unwrap();

    assert!(!app.peek, "nothing to uncover");
    assert!(!app.hide_meanings);
}

#[test]
fn ctrl_m_cycles_the_other_way() {
    let mut app = app_with_cards(3);
    app.enter_browse(DECK);
    app.browse_sel = 1;

    let back = KeyEvent::new(KeyCode::Char('m'), KeyModifiers::CONTROL);
    app.on_key(back).unwrap();
    assert_eq!(app.db.decks[0].cards[1].status, Study::Test);
    app.on_key(back).unwrap();
    assert_eq!(app.db.decks[0].cards[1].status, Study::Marked);
}

#[test]
fn m_cycles_the_status_while_reviewing() {
    let mut app = app_with_cards(4);
    app.db.decks[0].new_per_day = 100;
    app.start_session(DECK, Scope::Due);
    let id = app.session.as_ref().unwrap().current().unwrap();

    app.on_key(key('m')).unwrap();
    assert_eq!(app.db.decks[0].card(id).unwrap().status, Study::Review);
    app.on_key(key('m')).unwrap();
    assert_eq!(app.db.decks[0].card(id).unwrap().status, Study::Marked);
}

#[test]
fn status_command_sets_one_outright() {
    let mut app = app_with_cards(3);
    app.enter_browse(DECK);
    app.browse_sel = 0;

    app.run_command("status test");
    assert_eq!(app.db.decks[0].cards[0].status, Study::Test);

    app.run_command("status none");
    assert_eq!(app.db.decks[0].cards[0].status, Study::None);

    app.run_command("status nonsense");
    assert!(app.status.error);
    assert_eq!(
        app.db.decks[0].cards[0].status,
        Study::None,
        "a bad value changes nothing"
    );
}

#[test]
fn a_drill_can_target_one_status() {
    let mut app = app_with_cards(6);
    app.db.decks[0].cards[0].status = Study::Review;
    app.db.decks[0].cards[1].status = Study::Test;
    app.db.decks[0].cards[2].status = Study::Test;

    assert_eq!(app.build_queue(DECK, Scope::Flagged(None)).len(), 3);
    assert_eq!(
        app.build_queue(DECK, Scope::Flagged(Some(Study::Test)))
            .len(),
        2
    );
    assert_eq!(
        app.build_queue(DECK, Scope::Flagged(Some(Study::Marked)))
            .len(),
        0
    );
}

#[test]
fn the_browser_can_filter_to_one_status() {
    let mut app = app_with_cards(8);
    app.enter_browse(DECK);
    app.db.decks[0].cards[2].status = Study::Review;
    app.db.decks[0].cards[5].status = Study::Test;

    app.run_command("marked");
    assert_eq!(
        app.rows,
        vec![2, 5],
        "bare :marked shows everything flagged"
    );

    app.run_command("marked test");
    assert_eq!(app.rows, vec![5]);

    app.run_command("noh");
    assert_eq!(app.rows.len(), 8);
}

#[test]
fn a_miss_never_overwrites_a_status_you_set() {
    let mut app = app_with_cards(4);
    app.db.grading = Grading::Binary;
    app.db.decks[0].new_per_day = 100;
    app.start_session(DECK, Scope::Due);
    let id = app.session.as_ref().unwrap().current().unwrap();
    app.db
        .deck_by_id_mut(DECK)
        .unwrap()
        .card_mut(id)
        .unwrap()
        .status = Study::Test;

    app.answer(Grade::Again);
    assert_eq!(
        app.db.decks[0].card(id).unwrap().status,
        Study::Test,
        "your own label wins over the automatic mark"
    );
}

#[test]
fn an_old_collection_keeps_its_marks() {
    // Written before `status` existed: a plain `"marked": true`.
    let json = r#"{"decks":[{"id":1,"name":"d","cards":[
        {"id":1,"term":"a","definition":"x","due":"2026-01-01","marked":true},
        {"id":2,"term":"b","definition":"y","due":"2026-01-01","marked":false}
    ]}]}"#;
    let mut db: Db = serde_json::from_str(json).unwrap();
    db.migrate();

    assert_eq!(db.decks[0].cards[0].status, Study::Marked);
    assert_eq!(db.decks[0].cards[1].status, Study::None);

    // The status is written in the new shape, and the legacy boolean is
    // gone once it has been folded in.
    let round_trip = serde_json::to_string(&db).unwrap();
    assert!(round_trip.contains(r#""status":"marked""#), "{round_trip}");
    assert!(!round_trip.contains(r#""marked":"#), "{round_trip}");
}

#[test]
fn marked_filter_narrows_the_browser() {
    let mut app = app_with_cards(10);
    app.enter_browse(DECK);
    app.db.decks[0].cards[3].status = Study::Marked;
    app.db.decks[0].cards[7].status = Study::Marked;

    app.run_command("marked");
    assert_eq!(app.status_filter, StatusFilter::Any);
    assert_eq!(app.rows, vec![3, 7]);

    app.run_command("noh");
    assert_eq!(app.status_filter, StatusFilter::Off);
    assert_eq!(app.rows.len(), 10);
}

#[test]
fn marked_filter_composes_with_search() {
    let mut app = app_with_cards(20);
    app.enter_browse(DECK);
    for i in [1, 2, 11] {
        app.db.decks[0].cards[i].status = Study::Marked;
    }
    app.filter = "term1".to_string();
    app.status_filter = StatusFilter::Any;
    app.refresh_rows();
    // term1 and term11 match the text; term2 is marked but does not.
    assert_eq!(app.rows, vec![1, 11]);
}

#[test]
fn study_marked_starts_a_marked_session() {
    let mut app = app_with_cards(5);
    app.db.decks[0].cards[0].status = Study::Marked;
    app.run_command("study marked");

    let session = app.session.as_ref().expect("session started");
    // `:study marked` targets that one status; `M` drills everything flagged.
    assert_eq!(session.scope, Scope::Flagged(Some(Study::Marked)));
    assert_eq!(session.queue.len(), 1);
}

#[test]
fn study_marked_with_nothing_marked_reports_it() {
    let mut app = app_with_cards(5);
    app.run_command("study marked");
    assert!(app.session.is_none());
    assert!(app.status.text.contains("no marked words"));
}

#[test]
fn set_grading_switches_modes() {
    let mut app = app_with_cards(1);
    app.run_command("set grading=sm2");
    assert_eq!(app.db.grading, Grading::Sm2);

    app.run_command("set grading=binary");
    assert_eq!(app.db.grading, Grading::Binary);

    app.run_command("set grading=nonsense");
    assert!(app.status.error);
    assert_eq!(
        app.db.grading,
        Grading::Binary,
        "a bad value changes nothing"
    );
}

#[test]
fn y_and_n_grade_only_after_the_reveal() {
    let mut app = app_with_cards(4);
    app.db.grading = Grading::Binary;
    app.db.decks[0].new_per_day = 100;
    app.start_session(DECK, Scope::Due);

    app.on_key(key('n')).unwrap();
    assert_eq!(app.session.as_ref().unwrap().answered, 0);
    assert!(app.session.as_ref().unwrap().revealed, "it reveals instead");

    app.on_key(key('n')).unwrap();
    assert_eq!(app.session.as_ref().unwrap().answered, 1);
}

#[test]
fn y_grades_good_and_n_grades_again() {
    let mut app = app_with_cards(4);
    app.db.decks[0].new_per_day = 100;
    app.start_session(DECK, Scope::Due);

    app.on_key(key(' ')).unwrap();
    app.on_key(key('y')).unwrap();
    assert_eq!(app.db.log[0].grade, Grade::Good.index());

    app.on_key(key(' ')).unwrap();
    app.on_key(key('n')).unwrap();
    assert_eq!(app.db.log[1].grade, Grade::Again.index());
}

#[test]
fn dd_then_u_restores_the_card_in_place() {
    let mut app = app_with_cards(3);
    app.enter_browse(DECK);
    app.browse_sel = 1;

    app.on_key(key('d')).unwrap();
    app.on_key(key('d')).unwrap();
    assert_eq!(app.db.decks[0].cards.len(), 2);

    app.on_key(key('u')).unwrap();
    assert_eq!(app.db.decks[0].cards.len(), 3);
    assert_eq!(app.db.decks[0].cards[1].term, "term1");
}

#[test]
fn counts_prefix_motions() {
    let mut app = app_with_cards(20);
    app.enter_browse(DECK);
    app.on_key(key('5')).unwrap();
    app.on_key(key('j')).unwrap();
    assert_eq!(app.browse_sel, 5);
}

#[test]
fn braces_move_ten_rows_at_a_time() {
    let mut app = app_with_cards(40);
    app.enter_browse(DECK);
    app.on_key(key('}')).unwrap();
    assert_eq!(app.browse_sel, 10);
    app.on_key(key('2')).unwrap();
    app.on_key(key('}')).unwrap();
    assert_eq!(app.browse_sel, 30);
    app.on_key(key('{')).unwrap();
    assert_eq!(app.browse_sel, 20);
}

#[test]
fn gg_and_g_jump_to_the_ends() {
    let mut app = app_with_cards(20);
    app.enter_browse(DECK);
    app.on_key(key('G')).unwrap();
    assert_eq!(app.browse_sel, 19);

    app.on_key(key('g')).unwrap();
    app.on_key(key('g')).unwrap();
    assert_eq!(app.browse_sel, 0);
}

/// Type `text` into whatever is listening — unlike `typed`, without
/// opening the command line first.
fn keys(app: &mut App, text: &str) {
    for ch in text.chars() {
        app.on_key(key(ch)).unwrap();
    }
}

fn press(app: &mut App, code: KeyCode) {
    app.on_key(KeyEvent::new(code, KeyModifiers::NONE)).unwrap();
}

fn ctrl(app: &mut App, ch: char) {
    app.on_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL))
        .unwrap();
}

/// A browser over real words, for searches that need spelling to mean
/// something.
fn app_with_words(words: &[&str]) -> App {
    let mut app = app_with_cards(words.len());
    for (card, word) in app.db.decks[0].cards.iter_mut().zip(words) {
        card.term = word.to_string();
        card.definition = format!("the meaning of {word}");
    }
    app.enter_browse(DECK);
    app
}

/// The term under the browser's cursor.
fn selected_term(app: &App) -> String {
    let (_, index) = app.selected_card().expect("a selected card");
    app.db.decks[0].cards[index].term.clone()
}

#[test]
fn f_filters_rows() {
    let mut app = app_with_cards(15);
    app.enter_browse(DECK);
    app.on_key(key('f')).unwrap();
    assert_eq!(app.mode, Mode::Filter);
    keys(&mut app, "term1");
    // The list narrows as you type, before Enter.
    assert_eq!(app.rows.len(), 6, "term1, term10..term14");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.rows.len(), 6);
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn esc_puts_the_filter_back() {
    let mut app = app_with_cards(15);
    app.enter_browse(DECK);
    app.on_key(key('f')).unwrap();
    keys(&mut app, "term1");
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.rows.len(), 15);
    assert!(app.filter.is_empty());
}

#[test]
fn slash_moves_the_cursor_as_you_type() {
    let mut app = app_with_words(&["abash", "abate", "abdicate", "aberration", "abhor"]);
    app.on_key(key('/')).unwrap();
    assert_eq!(app.mode, Mode::Search);
    keys(&mut app, "abd");
    assert_eq!(selected_term(&app), "abdicate", "there before Enter");
    assert_eq!(app.rows.len(), 5, "and the list is left whole");
    // One slip from abdicate would still find it; two is no match.
    keys(&mut app, "qq");
    assert_eq!(app.browse_sel, 0, "no match: back where it started");
    press(&mut app, KeyCode::Backspace);
    press(&mut app, KeyCode::Backspace);
    press(&mut app, KeyCode::Enter);
    assert_eq!(selected_term(&app), "abdicate");
    assert_eq!(app.search, "abd");
}

#[test]
fn esc_puts_the_cursor_back_where_it_was() {
    let mut app = app_with_words(&["abash", "abate", "abdicate", "aberration", "abhor"]);
    app.on_key(key('j')).unwrap();
    app.on_key(key('/')).unwrap();
    keys(&mut app, "abh");
    assert_eq!(selected_term(&app), "abhor");
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.browse_sel, 1);
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn a_misspelt_search_still_lands_on_the_word() {
    let mut app = app_with_words(&["abash", "ubiquitous", "zealot", "obsequious"]);
    app.on_key(key('/')).unwrap();
    keys(&mut app, "ubiqitus");
    press(&mut app, KeyCode::Enter);
    assert_eq!(selected_term(&app), "ubiquitous");
    assert!(app.status.text.contains("close"), "{}", app.status.text);
}

#[test]
fn n_steps_through_the_matches_best_first() {
    // "abate" is the exact word; the others merely start with it or
    // mention it, so they come after whatever their place in the deck.
    let mut app = app_with_words(&["abated", "abash", "abate", "abatement"]);
    app.on_key(key('/')).unwrap();
    keys(&mut app, "abate");
    press(&mut app, KeyCode::Enter);
    assert_eq!(selected_term(&app), "abate");
    app.on_key(key('n')).unwrap();
    assert_eq!(selected_term(&app), "abated");
    app.on_key(key('n')).unwrap();
    assert_eq!(selected_term(&app), "abatement");
    app.on_key(key('n')).unwrap();
    assert_eq!(selected_term(&app), "abate", "and round again");
    app.on_key(key('N')).unwrap();
    assert_eq!(selected_term(&app), "abatement");
}

#[test]
fn moving_off_a_match_puts_the_highlight_away() {
    let mut app = app_with_words(&["abash", "abate", "abated"]);
    app.on_key(key('/')).unwrap();
    keys(&mut app, "abate");
    assert!(app.search_highlighted(), "while typing");
    press(&mut app, KeyCode::Enter);
    assert!(app.search_highlighted(), "on the match it landed on");
    app.on_key(key('j')).unwrap();
    assert!(!app.search_highlighted(), "gone once the cursor moves on");
    app.on_key(key('n')).unwrap();
    assert!(app.search_highlighted(), "n brings it back");
}

#[test]
fn esc_puts_the_highlight_away_before_leaving_the_list() {
    let mut app = app_with_words(&["abash", "abate"]);
    app.on_key(key('/')).unwrap();
    keys(&mut app, "abate");
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Browse, "the first Esc only hides it");
    assert!(!app.search_highlighted());
    app.on_key(key('n')).unwrap();
    assert_eq!(selected_term(&app), "abate", "the search is still there for n");
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Decks);
}

#[test]
fn noh_forgets_the_search() {
    let mut app = app_with_words(&["abash", "abate"]);
    app.on_key(key('/')).unwrap();
    keys(&mut app, "abate");
    press(&mut app, KeyCode::Enter);
    assert!(!app.search_matches().is_empty());
    app.run_command("noh");
    assert!(app.search_matches().is_empty());
}

#[test]
fn enter_opens_the_word_to_read_and_i_edits_it() {
    let mut app = app_with_words(&["abash", "abate", "abdicate"]);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.screen, Screen::Card);
    assert!(app.editor.is_none(), "reading, not editing");
    draw(&mut app, 80, 24);

    app.on_key(key('J')).unwrap();
    assert_eq!(selected_term(&app), "abate");
    press(&mut app, KeyCode::Tab);
    assert_eq!(selected_term(&app), "abdicate");
    app.on_key(key('J')).unwrap();
    assert_eq!(selected_term(&app), "abdicate", "the last word stays put");
    app.on_key(key('K')).unwrap();
    assert_eq!(selected_term(&app), "abate");

    app.on_key(key('i')).unwrap();
    assert!(app.editor.is_some());
    app.on_key(key('q')).unwrap();
    assert_eq!(app.screen, Screen::Card, "the editor closes back onto it");

    app.on_key(key('q')).unwrap();
    assert_eq!(app.screen, Screen::Browse);
    assert_eq!(
        selected_term(&app),
        "abate",
        "where J and K left the cursor"
    );
}

#[test]
fn the_word_view_scrolls_a_long_card() {
    let mut app = app_with_words(&["abash"]);
    app.db.decks[0].cards[0].example = (0..40)
        .map(|i| format!("example sentence number {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    press(&mut app, KeyCode::Enter);
    draw(&mut app, 80, 24);
    app.on_key(key('j')).unwrap();
    app.on_key(key('j')).unwrap();
    assert_eq!(app.card_scroll, 2, "j scrolls the card, not the list");
    app.on_key(key('G')).unwrap();
    draw(&mut app, 80, 24);
    assert!(app.card_scroll > 2, "G goes to the bottom");
    app.on_key(key('g')).unwrap();
    app.on_key(key('g')).unwrap();
    assert_eq!(app.card_scroll, 0, "gg to the top");
    assert_eq!(app.screen, Screen::Card);
}

/// Draw one frame and hand back what is on the screen, row by row.
fn screen_text(app: &mut App, width: u16, height: u16) -> String {
    let backend = ratatui::backend::TestBackend::new(width, height);
    let mut terminal = ratatui::Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| crate::ui::render(frame, app))
        .expect("draw");
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn set_reverse_flips_the_deck_and_can_be_undone() {
    let mut app = app_with_cards(1);
    app.run_command("set reverse=on");
    assert!(app.db.decks[0].reverse);
    app.run_command("set reverse=sideways");
    assert!(app.status.error, "only on or off");
    assert!(app.db.decks[0].reverse, "and a bad value changes nothing");
    app.on_key(key('u')).unwrap();
    assert!(!app.db.decks[0].reverse);
}

#[test]
fn a_reverse_card_asks_for_the_word_without_showing_it() {
    let mut app = app_with_words(&["abash"]);
    {
        let card = &mut app.db.decks[0].cards[0];
        card.pron = "uh BASH".into();
        card.example = "Meredith felt *abashed* by it.".into();
    }
    app.db.decks[0].reverse = true;
    app.start_session(DECK, Scope::Due);

    let question = screen_text(&mut app, 100, 30);
    assert!(question.contains("the meaning of abash"), "{question}");
    for giveaway in ["abash", "Meredith", "uh BASH"] {
        let shown = question.replace("the meaning of abash", "");
        assert!(
            !shown.contains(giveaway),
            "{giveaway:?} on the question side:\n{question}"
        );
    }

    app.on_key(key(' ')).unwrap();
    let answer = screen_text(&mut app, 100, 30);
    assert!(
        answer.contains("abash") && answer.contains("Meredith"),
        "{answer}"
    );
}

#[test]
fn p_will_not_say_the_answer_to_a_reverse_card() {
    let mut app = app_with_words(&["abash"]);
    app.db.decks[0].reverse = true;
    app.start_session(DECK, Scope::Due);
    app.on_key(key('p')).unwrap();
    assert!(app.status.error, "{}", app.status.text);
    assert!(app.status.text.contains("reveal"), "{}", app.status.text);
}

#[test]
fn set_voice_belongs_to_the_deck_and_blank_clears_it() {
    let mut app = app_with_cards(1);
    app.run_command("set voice=Kyoko");
    assert_eq!(app.db.decks[0].voice, "Kyoko");
    app.run_command("set voice=");
    assert_eq!(app.db.decks[0].voice, "");
}

#[test]
fn ctrl_c_explains_itself_instead_of_quitting() {
    let mut app = app_with_cards(3);
    app.enter_browse(DECK);
    ctrl(&mut app, 'c');
    assert!(!app.quit);
    assert!(app.status.text.contains(":q"), "{}", app.status.text);

    typed(&mut app, "dec");
    ctrl(&mut app, 'c');
    assert_eq!(
        app.mode,
        Mode::Normal,
        "it leaves the command line like Esc"
    );
}

#[test]
fn colon_command_creates_a_deck() {
    let mut app = app_with_cards(1);
    app.on_key(key(':')).unwrap();
    for ch in "deck greek".chars() {
        app.on_key(key(ch)).unwrap();
    }
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.db.decks.len(), 2);
    assert_eq!(app.db.decks[1].name, "greek");
}

#[test]
fn editor_writes_a_new_card() {
    let mut app = app_with_cards(1);
    app.enter_browse(DECK);
    app.on_key(key('a')).unwrap();
    assert_eq!(app.mode, Mode::Insert);

    for ch in "zeal".chars() {
        app.on_key(key(ch)).unwrap();
    }
    // Tab past pron and pos to the definition.
    for _ in 0..3 {
        app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .unwrap();
    }
    for ch in "eagerness".chars() {
        app.on_key(key(ch)).unwrap();
    }
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    app.on_key(key('Z')).unwrap();
    app.on_key(key('Z')).unwrap();

    assert!(app.editor.is_none());
    let card = app.db.decks[0].cards.last().unwrap();
    assert_eq!(card.term, "zeal");
    assert_eq!(card.definition, "eagerness");
}

#[test]
fn editor_refuses_a_card_without_a_definition() {
    let mut app = app_with_cards(1);
    app.enter_browse(DECK);
    app.open_new_card();
    app.field_mut(|f| f.insert_str("lonely"));
    assert!(!app.commit_editor());
    assert!(app.editor.is_some(), "editor stays open on error");
    assert!(app.status.error);
}

#[test]
fn review_space_reveals_then_grades() {
    let mut app = app_with_cards(3);
    app.db.decks[0].new_per_day = 100;
    app.start_session(DECK, Scope::Due);

    app.on_key(key(' ')).unwrap();
    assert!(app.session.as_ref().unwrap().revealed);

    app.on_key(key(' ')).unwrap();
    assert!(!app.session.as_ref().unwrap().revealed);
    assert_eq!(app.session.as_ref().unwrap().answered, 1);
}

#[test]
fn a_count_survives_the_first_key_of_an_operator() {
    // `5G` and `5gg` both used to lose the count: it was consumed before
    // the operator's second keystroke arrived.
    let mut app = app_with_cards(21);
    app.enter_browse(DECK);
    app.on_key(key('5')).unwrap();
    app.on_key(key('G')).unwrap();
    assert_eq!(app.browse_sel, 4, "5G selects row 5");

    app.on_key(key('1')).unwrap();
    app.on_key(key('2')).unwrap();
    app.on_key(key('g')).unwrap();
    app.on_key(key('g')).unwrap();
    assert_eq!(app.browse_sel, 11, "12gg selects row 12");
}

#[test]
fn bare_g_still_jumps_to_the_last_row() {
    let mut app = app_with_cards(9);
    app.enter_browse(DECK);
    app.on_key(key('G')).unwrap();
    assert_eq!(app.browse_sel, 8);
}

/// Height 15: a 1-line header, a 1-line status bar, and a panel whose
/// border and column header leave ten rows for cards.
const ROWS_VISIBLE: usize = 10;

#[test]
fn going_back_up_the_browser_does_not_keep_the_cursor_at_the_bottom() {
    let mut app = app_with_cards(30);
    app.enter_browse(DECK);
    draw(&mut app, 100, 15);
    assert_eq!(app.browse_off, 0, "a fresh list starts at the top");

    // Down past the window: it scrolls, and the cursor rides the last row.
    app.on_key(key('2')).unwrap();
    app.on_key(key('0')).unwrap();
    app.on_key(key('j')).unwrap();
    draw(&mut app, 100, 15);
    assert_eq!(app.browse_sel, 20);
    assert_eq!(app.browse_off, 20 + 1 - ROWS_VISIBLE);

    // Back up five rows. The cursor is still inside the window, so the
    // window stays where it is instead of redrawing from the top and
    // dragging the cursor back down to the bottom row.
    for _ in 0..5 {
        app.on_key(key('k')).unwrap();
    }
    draw(&mut app, 100, 15);
    assert_eq!(app.browse_sel, 15);
    assert_eq!(app.browse_off, 11, "the window did not follow");

    // Keep going and it scrolls again, now from the top row.
    for _ in 0..5 {
        app.on_key(key('k')).unwrap();
    }
    draw(&mut app, 100, 15);
    assert_eq!(app.browse_sel, 10);
    assert_eq!(app.browse_off, 10);
}

#[test]
fn the_deck_list_keeps_its_window_too() {
    let mut app = app_with_cards(1);
    for i in 0..30 {
        app.db.add_deck(format!("deck{i}"));
    }
    app.on_key(key('G')).unwrap();
    draw(&mut app, 100, 15);
    let last = app.db.decks.len() - 1;
    assert_eq!(app.decks_sel, last);
    assert_eq!(app.decks_off, last + 1 - ROWS_VISIBLE);

    app.on_key(key('k')).unwrap();
    draw(&mut app, 100, 15);
    assert_eq!(app.decks_off, last + 1 - ROWS_VISIBLE, "unchanged");
}

#[test]
fn a_count_no_other_key_uses_is_discarded() {
    let mut app = app_with_cards(12);
    app.enter_browse(DECK);
    app.on_key(key('7')).unwrap();
    app.on_key(key('m')).unwrap(); // mark takes no count
    assert!(
        app.count.is_empty(),
        "a stale count would affect the next motion"
    );
    app.on_key(key('j')).unwrap();
    assert_eq!(app.browse_sel, 1);
}

#[test]
fn qa_saves_rather_than_discarding_unsaved_answers() {
    let mut app = app_with_cards(31);
    app.db.decks[0].new_per_day = 100;
    app.start_session(DECK, Scope::Due);
    app.on_key(key(' ')).unwrap();
    app.on_key(key('y')).unwrap();
    assert!(app.dirty);

    app.run_command("qa");
    assert!(app.quit);
    assert!(!app.dirty, ":qa must not throw the session away");
    assert!(app.path.exists(), "the collection was written");
    std::fs::remove_file(&app.path).ok();
}

#[test]
fn qa_bang_asks_before_discarding() {
    let mut app = app_with_cards(32);
    app.db.decks[0].new_per_day = 100;
    app.start_session(DECK, Scope::Due);
    app.on_key(key(' ')).unwrap();
    app.on_key(key('y')).unwrap();

    app.run_command("qa!");
    assert!(!app.quit, "it waits for the confirmation");
    assert!(app.confirm.is_some());

    app.on_key(key('y')).unwrap();
    assert!(app.quit);
}

#[test]
fn deleting_a_deck_drops_every_handle_into_it() {
    let mut app = app_with_cards(4);
    app.db.decks[0].new_per_day = 100;
    app.enter_browse(DECK);
    app.on_key(key('d')).unwrap();
    app.on_key(key('d')).unwrap();
    app.start_session(DECK, Scope::Due);
    assert!(app.session.is_some());

    app.screen = Screen::Decks;
    app.decks_sel = 0;
    app.on_key(key('d')).unwrap();
    app.on_key(key('d')).unwrap();
    app.on_key(key('y')).unwrap();

    assert!(app.db.decks.is_empty());
    assert!(app.session.is_none(), "the session cannot outlive its deck");
    assert_eq!(app.browse_deck, 0);

    // The deck is gone, but not beyond recall: `u` is the last word on
    // every change, a deleted deck included.
    app.on_key(key('u')).unwrap();
    assert_eq!(app.db.decks.len(), 1, "u brings the deck back");
    assert_eq!(app.db.decks[0].cards.len(), 3, "as it was when it went");
}

#[test]
fn u_walks_back_through_every_kind_of_change() {
    let mut app = app_with_cards(3);
    app.enter_browse(DECK);

    app.on_key(key('x')).unwrap(); // suspend a card
    app.on_key(key('m')).unwrap(); // give it a status
    app.run_command("deck second"); // and make a deck
    assert_eq!(app.db.decks.len(), 2);

    app.on_key(key('u')).unwrap();
    assert_eq!(app.db.decks.len(), 1, "the deck goes");
    app.on_key(key('u')).unwrap();
    assert_eq!(
        app.db.decks[0].cards[0].status,
        Study::None,
        "then the status"
    );
    app.on_key(key('u')).unwrap();
    assert!(!app.db.decks[0].cards[0].suspended, "then the suspend");

    app.on_key(key('u')).unwrap();
    assert!(app.status.error, "and then there is nothing left to undo");
    assert_eq!(app.status.text, "already at oldest change");
}

#[test]
fn ctrl_r_puts_back_what_u_took() {
    let mut app = app_with_cards(3);
    app.enter_browse(DECK);
    app.on_key(key('d')).unwrap();
    app.on_key(key('d')).unwrap();
    assert_eq!(app.db.decks[0].cards.len(), 2);

    app.on_key(key('u')).unwrap();
    assert_eq!(app.db.decks[0].cards.len(), 3, "u restores the card");

    let redo = KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL);
    app.on_key(redo).unwrap();
    assert_eq!(app.db.decks[0].cards.len(), 2, "Ctrl-r deletes it again");

    app.on_key(redo).unwrap();
    assert!(app.status.error);
    assert_eq!(app.status.text, "already at newest change");
}

#[test]
fn a_change_after_an_undo_closes_off_the_redo() {
    let mut app = app_with_cards(3);
    app.enter_browse(DECK);
    app.on_key(key('d')).unwrap();
    app.on_key(key('d')).unwrap();
    app.on_key(key('u')).unwrap();

    // Vim's rule: doing something else makes the undone change unreachable.
    app.on_key(key('x')).unwrap();
    let redo = KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL);
    app.on_key(redo).unwrap();
    assert_eq!(
        app.db.decks[0].cards.len(),
        3,
        "the delete does not come back"
    );
    assert_eq!(app.status.text, "already at newest change");
}

#[test]
fn a_count_undoes_that_many_changes() {
    let mut app = app_with_cards(4);
    app.enter_browse(DECK);
    for _ in 0..3 {
        app.on_key(key('d')).unwrap();
        app.on_key(key('d')).unwrap();
    }
    assert_eq!(app.db.decks[0].cards.len(), 1);

    app.on_key(key('3')).unwrap();
    app.on_key(key('u')).unwrap();
    assert_eq!(
        app.db.decks[0].cards.len(),
        4,
        "3u takes back three deletes"
    );
    assert!(app.status.text.contains("3 changes"), "{}", app.status.text);
}

#[test]
fn a_refused_change_leaves_nothing_to_undo() {
    let mut app = app_with_cards(3);
    app.enter_browse(DECK);

    app.run_command("deck "); // no name — refused
    app.run_command("set grading=sideways"); // not a mode — refused
    app.on_key(key('u')).unwrap();
    assert_eq!(app.status.text, "already at oldest change");
}

#[test]
fn undo_goes_back_to_where_the_change_happened() {
    let mut app = app_with_cards(5);
    app.enter_browse(DECK);
    app.browse_sel = 3;
    app.on_key(key('d')).unwrap();
    app.on_key(key('d')).unwrap();

    app.screen = Screen::Decks;
    app.on_key(key('u')).unwrap();
    assert_eq!(app.screen, Screen::Browse, "back to the browser");
    assert_eq!(app.browse_sel, 3, "and to the row it happened on");
}

#[test]
fn the_undo_history_is_bounded() {
    let mut app = app_with_cards(1);
    app.enter_browse(DECK);
    for _ in 0..UNDO_DEPTH + 10 {
        app.on_key(key('x')).unwrap();
    }
    assert_eq!(app.undo.len(), UNDO_DEPTH, "the oldest steps fall off");
}

#[test]
fn the_review_log_stays_bounded() {
    let mut app = app_with_cards(33);
    app.db.decks[0].new_per_day = 100;
    for _ in 0..MAX_LOG {
        app.db.log.push(ReviewLog {
            date: app.today,
            deck_id: DECK,
            card_id: 1,
            grade: 2,
            drill: false,
            learning: false,
        });
    }
    app.start_session(DECK, Scope::Due);
    app.answer(Grade::Good);
    assert_eq!(app.db.log.len(), MAX_LOG);
    assert_eq!(
        app.db.log.last().map(|e| e.grade),
        Some(Grade::Good.index()),
        "the newest answer is the one kept"
    );
}

#[test]
fn the_clock_rolls_over_past_midnight() {
    let mut app = app_with_cards(2);
    app.today = Local::now().date_naive() - Duration::days(1);
    app.refresh_today();
    assert_eq!(app.today, Local::now().date_naive());
}

#[test]
fn stats_scrolls_instead_of_moving_a_cursor() {
    let mut app = app_with_cards(2);
    app.on_key(key('S')).unwrap();
    assert_eq!(app.screen, Screen::Stats);
    assert_eq!(app.help_scroll, 0);
    app.on_key(key('j')).unwrap();
    assert_eq!(app.help_scroll, 1);
    app.on_key(key('q')).unwrap();
    assert_eq!(app.screen, Screen::Decks, "S remembers where it came from");
}

#[test]
fn a_huge_count_does_not_scroll_backwards() {
    let mut app = app_with_cards(2);
    app.on_key(key('S')).unwrap();
    for ch in "99999".chars() {
        app.on_key(key(ch)).unwrap();
    }
    app.on_key(key('j')).unwrap();
    assert_eq!(app.help_scroll, i16::MAX as u16);
}

/// A scratch CSV, named after the test that asked for it.
fn scratch_csv(name: &str, contents: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("gram-import-{}-{name}.csv", std::process::id()));
    std::fs::write(&path, contents).unwrap();
    path
}

#[test]
fn a_failed_import_leaves_no_deck_behind() {
    let mut app = app_with_cards(2);
    let missing = std::env::temp_dir().join("gram-no-such-file.csv");
    let _ = std::fs::remove_file(&missing);

    app.cmd_import(&format!("{} Fresh", missing.display()));

    assert!(app.status.error, "the failure is reported");
    assert_eq!(app.db.decks.len(), 1, "no half-made deck is left over");
    assert!(app.db.deck_index("Fresh").is_none());
    assert!(!app.dirty, "nothing to save");
}

#[test]
fn a_failed_import_does_not_touch_the_target_deck() {
    let mut app = app_with_cards(2);
    let empty = scratch_csv("empty", "");
    let before = app.db.decks[0].cards.clone();

    app.cmd_import(&format!("{} test", empty.display()));

    assert!(app.status.error, "an empty file is an error");
    assert_eq!(app.db.decks[0].cards.len(), before.len());
    assert_eq!(app.db.decks[0].cards[0].term, before[0].term);
    assert!(!app.dirty);
}

#[test]
fn importing_into_a_new_name_creates_the_deck() {
    let mut app = app_with_cards(1);
    // A successful import saves, and `app_with_cards` names its file after
    // the card count — which another test may be holding.
    app.path = std::env::temp_dir().join(format!("gram-import-{}.json", std::process::id()));
    let csv = scratch_csv(
        "fresh",
        "term,definition\nabash,to embarrass\nabate,to subside\n",
    );

    app.cmd_import(&format!("{} Fresh", csv.display()));

    assert!(!app.status.error, "{}", app.status.text);
    let deck = app
        .db
        .deck_by_id(app.db.decks[app.decks_sel].id)
        .expect("the new deck is selected");
    assert_eq!(deck.name, "Fresh");
    assert_eq!(deck.cards.len(), 2);
    assert_ne!(deck.id, 0, "a committed deck gets a real id");
    assert_ne!(deck.id, DECK, "and not the existing deck's");

    let mut ids: Vec<u64> = app
        .db
        .decks
        .iter()
        .flat_map(|d| d.cards.iter().map(|c| c.id))
        .collect();
    let count = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), count, "card ids stay unique across decks");
}

#[test]
fn digits_do_not_grade_before_the_answer_is_shown() {
    let mut app = app_with_cards(3);
    app.db.decks[0].new_per_day = 100;
    app.start_session(DECK, Scope::Due);

    app.on_key(key('3')).unwrap();
    assert_eq!(app.session.as_ref().unwrap().answered, 0);
    assert!(app.session.as_ref().unwrap().revealed);
}

/// Answer the next `count` cards in the session "I know this".
fn know(app: &mut App, count: usize) {
    for _ in 0..count {
        app.on_key(key(' ')).unwrap();
        app.on_key(key('y')).unwrap();
    }
}

fn reviewed(app: &App) -> usize {
    app.db.decks[0].cards.iter().filter(|c| c.reps > 0).count()
}

#[test]
fn undoing_a_change_never_takes_back_the_answers_since() {
    let mut app = app_with_cards(5);
    app.enter_browse(DECK);
    app.on_key(key('m')).unwrap();
    assert_eq!(app.db.decks[0].cards[0].status, Study::Review);
    app.back();
    app.start_session(DECK, Scope::Due);
    know(&mut app, 3);
    app.on_key(key('q')).unwrap();

    app.on_key(key('u')).unwrap();
    assert_eq!(
        app.db.decks[0].cards[0].status,
        Study::None,
        "the status change is undone"
    );
    assert_eq!(reviewed(&app), 3, "and the three answers stay answered");
    assert_eq!(app.db.log.len(), 3, "and stay in the log");
    assert_eq!(
        app.db.decks[0].new_allowance(app.today),
        app.db.decks[0].new_per_day - 3,
        "the day's new cards stay spent"
    );

    // Redo puts the status back, still without disturbing the answers.
    app.on_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(app.db.decks[0].cards[0].status, Study::Review);
    assert_eq!(reviewed(&app), 3);
}

#[test]
fn a_mark_the_answer_made_survives_undo_but_yours_does_not() {
    let mut app = app_with_cards(2);
    app.db.grading = Grading::Binary;
    app.enter_browse(DECK);
    app.run_command("set new=5"); // a change to undo later
    app.back();
    app.start_session(DECK, Scope::Due);
    let missed = app.session.as_ref().and_then(|s| s.current()).unwrap();
    app.on_key(key(' ')).unwrap();
    app.on_key(key('n')).unwrap(); // marks it ★
    app.on_key(key('q')).unwrap();

    app.on_key(key('u')).unwrap();
    let card = app.db.decks[0].card(missed).unwrap();
    assert_eq!(
        card.status,
        Study::Marked,
        "the miss's ★ is part of the answer"
    );
    assert_eq!(card.lapses, 0, "a first miss is not a lapse");
    assert_eq!(card.state, CardState::Learning);
}

#[test]
fn taking_an_answer_back_reaches_the_snapshots_too() {
    let mut app = app_with_cards(3);
    app.run_command("set new=5");
    app.start_session(DECK, Scope::Due);
    know(&mut app, 1);
    app.on_key(key('u')).unwrap(); // the answer, inside the session
    app.on_key(key('q')).unwrap();
    app.on_key(key('u')).unwrap(); // then the setting, from the history
    assert_eq!(reviewed(&app), 0, "nothing answered, before or after");
    assert!(app.db.log.is_empty());
}

#[test]
fn suspending_in_review_can_be_undone_afterwards() {
    let mut app = app_with_cards(3);
    app.start_session(DECK, Scope::Due);
    app.on_key(key('x')).unwrap();
    app.on_key(key('q')).unwrap();
    assert_eq!(
        app.db.decks[0].cards.iter().filter(|c| c.suspended).count(),
        1
    );
    app.on_key(key('u')).unwrap();
    assert!(app.db.decks[0].cards.iter().all(|c| !c.suspended));
}

#[test]
fn snapshots_leave_the_review_log_out() {
    let mut app = app_with_cards(2);
    app.start_session(DECK, Scope::Due);
    know(&mut app, 2);
    assert_eq!(app.db.log.len(), 2);
    app.run_command("set new=5");
    assert!(app.undo.last().is_some_and(|step| step.db.log.is_empty()));
    app.on_key(key('u')).unwrap();
    assert_eq!(app.db.log.len(), 2, "undo keeps the log it found");
}

#[test]
fn the_deck_list_says_how_many_words_are_still_unseen() {
    let mut app = app_with_cards(40);
    app.db.decks[0].new_per_day = 5;
    let screen = screen_text(&mut app, 100, 12);
    assert!(
        !screen.contains("unseen — n to learn"),
        "today's ration is still waiting, so NEW already says so:\n{screen}"
    );
    app.db.decks[0].spend_new(app.today, 5);
    let screen = screen_text(&mut app, 100, 12);
    assert!(
        screen.contains("40 unseen — n to learn"),
        "a spent ration should not read as a finished deck:\n{screen}"
    );
}

/// What the focused field holds, and where its cursor sits.
fn field_state(app: &App) -> (&str, usize) {
    let editor = app.editor.as_ref().expect("editor open");
    let field = &editor.fields[editor.field];
    (field.value(), field.cursor())
}

#[test]
fn editing_opens_with_the_cursor_on_a_character() {
    // The cursor used to open one past the last character, where `x` and `D`
    // have nothing under them: the first keystroke of an edit was swallowed.
    let mut app = app_with_cards(1);
    app.enter_browse(DECK);
    app.on_key(key('i')).unwrap();
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(field_state(&app), ("term0", 4));

    app.on_key(key('x')).unwrap();
    assert_eq!(field_state(&app), ("term", 3), "x bites on the first press");
}

#[test]
fn x_keeps_deleting_at_the_end_of_a_field() {
    let mut app = app_with_cards(1);
    app.enter_browse(DECK);
    app.on_key(key('i')).unwrap();
    for expected in ["term", "ter", "te", "t", ""] {
        app.on_key(key('x')).unwrap();
        assert_eq!(field_state(&app).0, expected);
    }
    app.on_key(key('x')).unwrap();
    assert_eq!(field_state(&app), ("", 0), "and stops at an empty field");
}

#[test]
fn moving_between_fields_keeps_the_cursor_on_a_character() {
    let mut app = app_with_cards(1);
    app.enter_browse(DECK);
    app.on_key(key('i')).unwrap();
    // Term, Pron, Pos, then Definition.
    for _ in 0..3 {
        app.on_key(key('j')).unwrap();
    }
    assert_eq!(field_state(&app), ("definition 0", 11));
    app.on_key(key('x')).unwrap();
    assert_eq!(field_state(&app).0, "definition ");

    press(&mut app, KeyCode::BackTab);
    press(&mut app, KeyCode::Tab);
    app.on_key(key('x')).unwrap();
    assert_eq!(field_state(&app).0, "definition", "Tab clamps too");
}

#[test]
fn leaving_insert_mode_then_wandering_fields_still_deletes() {
    let mut app = app_with_cards(1);
    app.enter_browse(DECK);
    app.on_key(key('i')).unwrap();
    app.on_key(key('A')).unwrap();
    assert_eq!(app.mode, Mode::Insert);
    for ch in "ish".chars() {
        app.on_key(key(ch)).unwrap();
    }
    press(&mut app, KeyCode::Esc);
    assert_eq!(field_state(&app), ("term0ish", 7));
    app.on_key(key('x')).unwrap();
    assert_eq!(field_state(&app).0, "term0is");
}

#[test]
fn w_and_d_leave_the_cursor_where_vim_does() {
    let mut app = app_with_cards(1);
    app.enter_browse(DECK);
    app.db.decks[0].cards[0].definition = "to make ashamed".into();
    app.refresh_rows();
    app.on_key(key('i')).unwrap();
    for _ in 0..3 {
        app.on_key(key('j')).unwrap();
    }
    app.on_key(key('0')).unwrap();
    app.on_key(key('w')).unwrap();
    app.on_key(key('w')).unwrap();
    assert_eq!(field_state(&app), ("to make ashamed", 8));
    app.on_key(key('w')).unwrap();
    assert_eq!(field_state(&app).1, 14, "w stops on the last character");

    app.on_key(key('0')).unwrap();
    app.on_key(key('w')).unwrap();
    app.on_key(key('D')).unwrap();
    assert_eq!(field_state(&app), ("to ", 2), "D leaves the cursor on text");
    app.on_key(key('x')).unwrap();
    assert_eq!(field_state(&app).0, "to");
}

#[test]
fn arrows_move_between_fields_without_leaving_insert_mode() {
    let mut app = app_with_cards(1);
    app.enter_browse(DECK);
    app.on_key(key('a')).unwrap();
    assert_eq!(app.mode, Mode::Insert);
    for ch in "zeal".chars() {
        app.on_key(key(ch)).unwrap();
    }
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Down);
    assert_eq!(app.mode, Mode::Insert, "still typing");
    assert_eq!(app.editor.as_ref().unwrap().field, 3, "on the definition");
    for ch in "eagerness".chars() {
        app.on_key(key(ch)).unwrap();
    }
    press(&mut app, KeyCode::Up);
    press(&mut app, KeyCode::Up);
    press(&mut app, KeyCode::Up);
    assert_eq!(
        field_state(&app),
        ("zeal", 4),
        "back on the term, at its end"
    );
}

#[test]
fn a_failed_save_does_not_write_the_collection() {
    let mut app = app_with_cards(1);
    app.enter_browse(DECK);
    app.open_new_card();
    app.field_mut(|f| f.insert_str("lonely"));
    // ZZ with no definition: the editor stays open and nothing is written,
    // so the complaint about it is still on screen.
    press(&mut app, KeyCode::Esc);
    app.on_key(key('Z')).unwrap();
    app.on_key(key('Z')).unwrap();
    assert!(app.editor.is_some());
    assert!(app.status.error, "{}", app.status.text);
    assert!(!app.path.exists(), "nothing saved");
}

#[test]
fn a_half_typed_operator_does_not_escape_the_editor() {
    let mut app = app_with_cards(2);
    app.enter_browse(DECK);
    app.on_key(key('i')).unwrap();
    app.on_key(key('d')).unwrap();
    assert_eq!(app.pending, Some('d'));
    app.on_key(key('q')).unwrap();
    assert_eq!(app.pending, None, "cancelling the editor cancels the dd");
    app.on_key(key('d')).unwrap();
    assert_eq!(app.db.decks[0].cards.len(), 2, "no card deleted");
}
