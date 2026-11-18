//! How much of something is done. A project's and a milestone's progress are counted from their
//! tasks the same way everywhere, so the server's numbers and the client's bars always agree.

use serde::{Deserialize, Serialize};

use crate::Status;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    /// Tasks counted.
    pub total: u32,
    /// Of those, the ones done.
    pub done: u32,
}

impl Progress {
    /// Count a list of task states.
    pub fn of(states: impl IntoIterator<Item = Status>) -> Progress {
        let mut p = Progress::default();
        for s in states {
            p.add(s);
        }
        p
    }

    pub fn add(&mut self, status: Status) {
        self.total = self.total.saturating_add(1);
        if status.is_done() {
            self.done = self.done.saturating_add(1);
        }
    }

    /// The share done, from 0.0 to 1.0. Nothing to do counts as nothing done.
    pub fn fraction(self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            (self.done.min(self.total) as f32) / (self.total as f32)
        }
    }

    /// Whole percent, rounded down so 99.6 % never shows as finished.
    pub fn percent(self) -> u32 {
        if self.total == 0 {
            return 0;
        }
        let done = u64::from(self.done.min(self.total));
        u32::try_from(done * 100 / u64::from(self.total)).unwrap_or(100)
    }

    /// Every task is done (and there is at least one).
    pub fn complete(self) -> bool {
        self.total > 0 && self.done >= self.total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_counts_done_tasks() {
        let p = Progress::of([Status::Done, Status::Todo, Status::InReview, Status::Done]);
        assert_eq!(p, Progress { total: 4, done: 2 });
        assert_eq!(p.percent(), 50);
        assert!((p.fraction() - 0.5).abs() < f32::EPSILON);
        assert!(!p.complete());
    }

    #[test]
    fn nothing_to_do_is_not_finished_and_rounding_never_reaches_100_early() {
        assert_eq!(Progress::default().percent(), 0);
        assert!(!Progress::default().complete());
        let almost = Progress {
            total: 1000,
            done: 999,
        };
        assert_eq!(almost.percent(), 99);
        assert!(Progress { total: 3, done: 3 }.complete());
    }
}
