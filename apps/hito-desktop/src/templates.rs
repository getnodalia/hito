//! Starting points for a new project: a few milestones and tasks, written in the person's
//! language when the project is created. Their text is sealed like any other.

use crate::i18n::tr;
use crate::net::Template;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Blank,
    Launch,
    Client,
    Sprint,
    Event,
}

impl Kind {
    pub const ALL: [Kind; 5] = [
        Kind::Blank,
        Kind::Launch,
        Kind::Client,
        Kind::Sprint,
        Kind::Event,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Kind::Blank => tr("Blank project"),
            Kind::Launch => tr("Product launch"),
            Kind::Client => tr("Client project"),
            Kind::Sprint => tr("Two-week sprint"),
            Kind::Event => tr("Event"),
        }
    }

    pub fn summary(self) -> &'static str {
        match self {
            Kind::Blank => tr("An empty board."),
            Kind::Launch => tr("Plan, build, launch: three milestones."),
            Kind::Client => tr("From the kickoff to the handover."),
            Kind::Sprint => tr("A short cycle with a review at the end."),
            Kind::Event => tr("Venue, guests and the day itself."),
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Kind::Blank => "plus",
            Kind::Launch => "flag",
            Kind::Client => "users",
            Kind::Sprint => "clock",
            Kind::Event => "calendar",
        }
    }

    /// The milestones and tasks, in the current language.
    pub fn contents(self) -> Template {
        let s = |list: &[&str]| list.iter().map(|t| t.to_string()).collect::<Vec<_>>();
        let tasks = |list: &[(&str, usize)]| {
            list.iter()
                .map(|(t, m)| (t.to_string(), Some(*m)))
                .collect::<Vec<_>>()
        };
        match self {
            Kind::Blank => Template::default(),
            Kind::Launch => Template {
                milestones: s(&[tr("Plan"), tr("Build"), tr("Launch")]),
                tasks: tasks(&[
                    (tr("Agree on the goals and the date"), 0),
                    (tr("Write the brief"), 0),
                    (tr("Build the first version"), 1),
                    (tr("Test with five customers"), 1),
                    (tr("Prepare the announcement"), 2),
                    (tr("Launch"), 2),
                ]),
            },
            Kind::Client => Template {
                milestones: s(&[tr("Kickoff"), tr("Delivery"), tr("Handover")]),
                tasks: tasks(&[
                    (tr("Kickoff meeting"), 0),
                    (tr("Confirm the scope and budget"), 0),
                    (tr("First delivery"), 1),
                    (tr("Client review"), 1),
                    (tr("Final changes"), 2),
                    (tr("Send the invoice"), 2),
                ]),
            },
            Kind::Sprint => Template {
                milestones: s(&[tr("Sprint review")]),
                tasks: tasks(&[
                    (tr("Sprint planning"), 0),
                    (tr("Daily check-ins"), 0),
                    (tr("Demo"), 0),
                    (tr("Retrospective"), 0),
                ]),
            },
            Kind::Event => Template {
                milestones: s(&[tr("Preparation"), tr("The day")]),
                tasks: tasks(&[
                    (tr("Book the venue"), 0),
                    (tr("Send the invitations"), 0),
                    (tr("Order the catering"), 0),
                    (tr("Set up the room"), 1),
                    (tr("Thank the guests"), 1),
                ]),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_task_points_at_one_of_its_milestones() {
        for kind in Kind::ALL {
            let t = kind.contents();
            for (_, m) in &t.tasks {
                assert!(m.is_none_or(|i| i < t.milestones.len()));
            }
        }
    }
}
