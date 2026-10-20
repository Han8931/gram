//! The numbers behind the statistics screen.

use super::*;

impl App {
    /// Reviews per day over the last `days` days, oldest first.
    pub fn review_history(&self, days: usize) -> Vec<(NaiveDate, u64)> {
        (0..days)
            .rev()
            .map(|back| {
                let date = self.today - Duration::days(back as i64);
                let count = self.db.log.iter().filter(|e| e.date == date).count() as u64;
                (date, count)
            })
            .collect()
    }

    /// Cards falling due on each of the next `days` days.
    pub fn due_forecast(&self, days: usize) -> Vec<(NaiveDate, u64)> {
        (0..days)
            .map(|ahead| {
                let date = self.today + Duration::days(ahead as i64);
                let count = self
                    .db
                    .decks
                    .iter()
                    .flat_map(|d| d.cards.iter())
                    .filter(|c| {
                        !c.suspended && c.state != CardState::New && {
                            if ahead == 0 {
                                c.due <= date
                            } else {
                                c.due == date
                            }
                        }
                    })
                    .count() as u64;
                (date, count)
            })
            .collect()
    }

    /// Share of every answer that was not `Again` — drills, same-session
    /// retries and first sightings of new words included. A measure of how the
    /// sessions went, not of what you remember.
    pub fn answer_accuracy(&self) -> Option<f32> {
        let total = self.db.log.len();
        if total == 0 {
            return None;
        }
        let good = self
            .db
            .log
            .iter()
            .filter(|e| e.grade != Grade::Again.index())
            .count();
        Some(good as f32 / total as f32)
    }

    /// Share of scheduled reviews answered right the first time: the number
    /// that actually says whether the schedule is working.
    ///
    /// Counts one answer per card per day — the first — and leaves out drills,
    /// which ignore due dates by design, and words being seen for the first
    /// time, which is learning rather than remembering. Collections logged
    /// before those two were recorded count as scheduled reviews, which is
    /// what they were for anyone not drilling.
    pub fn review_retention(&self) -> Option<f32> {
        let mut seen: HashSet<(u64, NaiveDate)> = HashSet::new();
        let mut total = 0usize;
        let mut good = 0usize;
        for entry in &self.db.log {
            if entry.drill || entry.learning {
                continue;
            }
            // A retry later the same day is the same question asked twice.
            if !seen.insert((entry.card_id, entry.date)) {
                continue;
            }
            total += 1;
            if entry.grade != Grade::Again.index() {
                good += 1;
            }
        }
        (total > 0).then(|| good as f32 / total as f32)
    }
}
