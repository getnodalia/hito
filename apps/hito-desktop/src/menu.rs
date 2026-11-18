//! The menus Hito shows in the desktop's top bar (the Linux global menu), as (label, action id)
//! pairs. `-` is a separator. Every id maps to an action in `Action::from_menu`.

use crate::i18n::tr;

pub type Entry = (&'static str, &'static str);

pub fn tree() -> Vec<(&'static str, Vec<Entry>)> {
    vec![
        (
            "Hito",
            vec![
                (tr("About Hito"), "about"),
                ("-", ""),
                (tr("Preferences"), "preferences"),
                (tr("Keyboard shortcuts"), "shortcuts"),
                (tr("Check for updates…"), "updates"),
                ("-", ""),
                (tr("Log out"), "logout"),
                (tr("Quit"), "quit"),
            ],
        ),
        (
            tr("File"),
            vec![
                (tr("Home"), "home"),
                ("-", ""),
                (tr("New task"), "new-task"),
                (tr("New project"), "new-project"),
                ("-", ""),
                (tr("API tokens and webhooks"), "integrations"),
            ],
        ),
        (
            tr("View"),
            vec![
                (tr("Overview"), "overview"),
                (tr("My tasks"), "my-tasks"),
                ("-", ""),
                (tr("Board"), "board"),
                (tr("List"), "list"),
                (tr("Milestones"), "milestones"),
                (tr("Activity"), "activity"),
                ("-", ""),
                (tr("Refresh"), "refresh"),
            ],
        ),
    ]
}
