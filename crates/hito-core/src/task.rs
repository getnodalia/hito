//! A task's state and priority.

use serde::{Deserialize, Serialize};

/// Where a task is in its life. The board shows one column per state, in this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Todo,
    InProgress,
    InReview,
    Done,
}

impl Status {
    pub const ALL: [Status; 4] = [
        Status::Todo,
        Status::InProgress,
        Status::InReview,
        Status::Done,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Status::Todo => "todo",
            Status::InProgress => "in_progress",
            Status::InReview => "in_review",
            Status::Done => "done",
        }
    }

    pub fn parse(s: &str) -> Option<Status> {
        Some(match s {
            "todo" => Status::Todo,
            "in_progress" => Status::InProgress,
            "in_review" => Status::InReview,
            "done" => Status::Done,
            _ => return None,
        })
    }

    pub const fn is_done(self) -> bool {
        matches!(self, Status::Done)
    }
}

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    #[default]
    Normal,
    High,
    Urgent,
}

impl Priority {
    pub const ALL: [Priority; 4] = [
        Priority::Low,
        Priority::Normal,
        Priority::High,
        Priority::Urgent,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Priority::Low => "low",
            Priority::Normal => "normal",
            Priority::High => "high",
            Priority::Urgent => "urgent",
        }
    }

    pub fn parse(s: &str) -> Option<Priority> {
        Some(match s {
            "low" => Priority::Low,
            "normal" => Priority::Normal,
            "high" => Priority::High,
            "urgent" => Priority::Urgent,
            _ => return None,
        })
    }
}

/// Whether a task is overdue at `now` (milliseconds): it has a due time in the past and is not
/// done.
pub fn overdue(status: Status, due_at: Option<i64>, now: i64) -> bool {
    !status.is_done() && due_at.is_some_and(|d| d < now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_and_priorities_round_trip_through_their_names() {
        for s in Status::ALL {
            assert_eq!(Status::parse(s.as_str()), Some(s));
        }
        for p in Priority::ALL {
            assert_eq!(Priority::parse(p.as_str()), Some(p));
        }
        assert_eq!(Status::parse("doing"), None);
        assert_eq!(Priority::parse(""), None);
    }

    #[test]
    fn only_open_tasks_past_their_date_are_overdue() {
        assert!(overdue(Status::Todo, Some(10), 11));
        assert!(!overdue(Status::Done, Some(10), 11));
        assert!(!overdue(Status::InProgress, Some(12), 11));
        assert!(!overdue(Status::InProgress, None, 11));
    }
}
