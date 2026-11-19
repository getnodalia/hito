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
    Hiring,
    Onboarding,
    Campaign,
    Website,
    Quarter,
}

impl Kind {
    pub const ALL: [Kind; 10] = [
        Kind::Blank,
        Kind::Launch,
        Kind::Client,
        Kind::Sprint,
        Kind::Event,
        Kind::Hiring,
        Kind::Onboarding,
        Kind::Campaign,
        Kind::Website,
        Kind::Quarter,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Kind::Blank => tr("Blank project"),
            Kind::Launch => tr("Product launch"),
            Kind::Client => tr("Client project"),
            Kind::Sprint => tr("Two-week sprint"),
            Kind::Event => tr("Event"),
            Kind::Hiring => tr("Hiring"),
            Kind::Onboarding => tr("Onboarding"),
            Kind::Campaign => tr("Marketing campaign"),
            Kind::Website => tr("Website redesign"),
            Kind::Quarter => tr("Quarter plan"),
        }
    }

    pub fn summary(self) -> &'static str {
        match self {
            Kind::Blank => tr("An empty board."),
            Kind::Launch => tr("Plan, build, launch: three milestones."),
            Kind::Client => tr("From the kickoff to the handover."),
            Kind::Sprint => tr("A short cycle with a review at the end."),
            Kind::Event => tr("Venue, guests and the day itself."),
            Kind::Hiring => tr("From the job post to the first day."),
            Kind::Onboarding => tr("A new colleague's first weeks."),
            Kind::Campaign => tr("Idea, production and the weeks it runs."),
            Kind::Website => tr("Structure, design and launch of a site."),
            Kind::Quarter => tr("Goals, work and review for three months."),
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Kind::Blank => "plus",
            Kind::Launch => "flag",
            Kind::Client => "users",
            Kind::Sprint => "clock",
            Kind::Event => "calendar",
            Kind::Hiring => "search",
            Kind::Onboarding => "home",
            Kind::Campaign => "mail",
            Kind::Website => "globe",
            Kind::Quarter => "layers",
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
            Kind::Hiring => Template {
                milestones: s(&[tr("Open the role"), tr("Interviews"), tr("Offer")]),
                tasks: tasks(&[
                    (tr("Agree on what the role needs"), 0),
                    (tr("Write and publish the job post"), 0),
                    (tr("Screen the applications"), 1),
                    (tr("Hold the interviews"), 1),
                    (tr("Check references"), 2),
                    (tr("Send the offer"), 2),
                ]),
            },
            Kind::Onboarding => Template {
                milestones: s(&[tr("Before day one"), tr("First week"), tr("First month")]),
                tasks: tasks(&[
                    (tr("Prepare the accounts and the desk"), 0),
                    (tr("Send the welcome message"), 0),
                    (tr("Introduce the team"), 1),
                    (tr("Walk through the tools"), 1),
                    (tr("Agree on the first goals"), 2),
                    (tr("Hold the one-month check-in"), 2),
                ]),
            },
            Kind::Campaign => Template {
                milestones: s(&[tr("Idea"), tr("Production"), tr("Live")]),
                tasks: tasks(&[
                    (tr("Define the audience and the message"), 0),
                    (tr("Set the budget"), 0),
                    (tr("Write the copy"), 1),
                    (tr("Prepare the images"), 1),
                    (tr("Schedule the posts"), 2),
                    (tr("Report the results"), 2),
                ]),
            },
            Kind::Website => Template {
                milestones: s(&[tr("Structure"), tr("Design"), tr("Launch")]),
                tasks: tasks(&[
                    (tr("List the pages and what each is for"), 0),
                    (tr("Review the old content"), 0),
                    (tr("Design the main pages"), 1),
                    (tr("Test it on a phone"), 1),
                    (tr("Redirect the old addresses"), 2),
                    (tr("Publish"), 2),
                ]),
            },
            Kind::Quarter => Template {
                milestones: s(&[tr("Goals"), tr("Progress review"), tr("Wrap-up")]),
                tasks: tasks(&[
                    (tr("Choose three goals"), 0),
                    (tr("Assign an owner to each goal"), 0),
                    (tr("Review progress at the halfway point"), 1),
                    (tr("Drop what is not working"), 1),
                    (tr("Write the closing summary"), 2),
                    (tr("Share it with the team"), 2),
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

    #[test]
    fn every_template_has_a_name_a_summary_an_icon_and_work_in_every_milestone() {
        let mut names = std::collections::HashSet::new();
        for kind in Kind::ALL {
            assert!(names.insert(kind.name()), "{kind:?} repeats a name");
            assert!(
                !kind.summary().is_empty() && !kind.icon().is_empty(),
                "{kind:?}"
            );
            let t = kind.contents();
            if kind == Kind::Blank {
                assert!(t.milestones.is_empty() && t.tasks.is_empty());
                continue;
            }
            for i in 0..t.milestones.len() {
                assert!(
                    t.tasks.iter().any(|(_, m)| *m == Some(i)),
                    "{kind:?}: milestone {i} has no task"
                );
            }
        }
    }

    #[test]
    fn the_new_templates_read_in_spanish() {
        crate::i18n::set(crate::i18n::Lang::Es);
        assert_eq!(Kind::Hiring.name(), "Contratación");
        assert_eq!(Kind::Quarter.contents().milestones[1], "Revisión de avance");
        assert_eq!(Kind::Quarter.contents().milestones[2], "Cierre");
        for kind in Kind::ALL {
            let t = kind.contents();
            for title in t
                .milestones
                .iter()
                .chain(t.tasks.iter().map(|(title, _)| title))
            {
                assert!(!title.is_empty(), "{kind:?}");
            }
        }
    }
}
