//! Modal dialogs: create or join a workspace, invite people, a new project (with its template),
//! a new task, a milestone, a project's settings, the integrations (API tokens and webhooks for
//! Ágora and other apps) and the two confirmations. Each has the NDL heading with a round close
//! button. The small form helpers here are shared with the task panel.

use egui::{vec2, Id, RichText, TextEdit, Ui};
use hito_core::{MilestoneId, Priority, ProjectId, Role, Status, TaskId, UserId};
use hito_protocol::{ActivityKind, Invite};
use nodalia_theme::creator::{self as cr, layout, Look};
use nodalia_theme::font_size;
use nodalia_theme::for_egui::color;

use crate::app::{Action, Main};
use crate::dates;
use crate::i18n::{tr, trf};
use crate::pages::{priority_name, status_name};
use crate::settings::Settings;
use crate::templates::Kind;

/// The integrations dialog: this person's API tokens and the workspace's webhooks.
#[derive(Clone, Debug, Default)]
pub struct Integrations {
    /// A token or webhook secret just created, shown once.
    pub secret: Option<String>,
    pub token_name: String,
    pub hook_name: String,
    pub hook_url: String,
    /// The key the receiver gave, when it hands one out; empty lets the server choose.
    pub hook_secret: String,
    pub hook_project: Option<ProjectId>,
    /// The activity to send; empty sends all of it.
    pub hook_events: Vec<ActivityKind>,
}

#[derive(Clone, Debug)]
pub enum Dialog {
    CreateWorkspace {
        name: String,
    },
    JoinWorkspace {
        code: String,
    },
    Invite {
        invite: Option<Invite>,
        guest: bool,
        uses: u32,
        hours: u32,
    },
    NewProject {
        name: String,
        description: String,
        template: Kind,
    },
    NewTask {
        project: ProjectId,
        title: String,
        status: Status,
        priority: Priority,
        assignee: Option<UserId>,
        milestone: Option<MilestoneId>,
        due: String,
    },
    Milestone {
        project: ProjectId,
        id: Option<MilestoneId>,
        name: String,
        description: String,
        due: String,
    },
    ProjectSettings {
        id: ProjectId,
        name: String,
        description: String,
        archived: bool,
    },
    Integrations(Integrations),
    ConfirmDeleteTask {
        id: TaskId,
    },
    ConfirmDeleteMilestone {
        id: MilestoneId,
    },
}

impl Dialog {
    pub fn new_project() -> Dialog {
        Dialog::NewProject {
            name: String::new(),
            description: String::new(),
            template: Kind::Blank,
        }
    }

    pub fn new_task(project: ProjectId) -> Dialog {
        Dialog::NewTask {
            project,
            title: String::new(),
            status: Status::Todo,
            priority: Priority::Normal,
            assignee: None,
            milestone: None,
            due: String::new(),
        }
    }

    pub fn milestone(
        project: ProjectId,
        id: Option<MilestoneId>,
        name: &str,
        description: &str,
        due: Option<i64>,
    ) -> Dialog {
        Dialog::Milestone {
            project,
            id,
            name: name.to_string(),
            description: description.to_string(),
            due: due.map(dates::date_text).unwrap_or_default(),
        }
    }

    pub fn invite() -> Dialog {
        Dialog::Invite {
            invite: None,
            guest: false,
            uses: 5,
            hours: 168,
        }
    }

    pub fn integrations() -> Dialog {
        Dialog::Integrations(Integrations::default())
    }

    fn title(&self) -> &'static str {
        match self {
            Dialog::CreateWorkspace { .. } => tr("Create a workspace"),
            Dialog::JoinWorkspace { .. } => tr("Join a workspace"),
            Dialog::Invite { .. } => tr("Invite people"),
            Dialog::NewProject { .. } => tr("New project"),
            Dialog::NewTask { .. } => tr("New task"),
            Dialog::Milestone { id: None, .. } => tr("New milestone"),
            Dialog::Milestone { .. } => tr("Edit milestone"),
            Dialog::ProjectSettings { .. } => tr("Project settings"),
            Dialog::Integrations(_) => tr("API tokens and webhooks"),
            Dialog::ConfirmDeleteTask { .. } => tr("Delete this task?"),
            Dialog::ConfirmDeleteMilestone { .. } => tr("Delete this milestone?"),
        }
    }

    fn width(&self) -> f32 {
        match self {
            Dialog::NewProject { .. } | Dialog::Integrations(_) => 520.0,
            _ => 400.0,
        }
    }
}

pub fn show(
    ctx: &egui::Context,
    main: &mut Main,
    _settings: &mut Settings,
    look: &Look,
    actions: &mut Vec<Action>,
) {
    let Some(mut dialog) = main.dialog.take() else {
        return;
    };
    let p = &look.palette;
    let frame = cr::island(look)
        .inner_margin(20)
        .stroke(egui::Stroke::new(1.0, color(p.border_subtle)));
    let mut close = false;
    let response = egui::Modal::new(Id::unique("hito-dialog"))
        .frame(frame)
        .show(ctx, |ui| {
            ui.set_width(dialog.width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(dialog.title())
                        .size(font_size::TITLE)
                        .strong()
                        .color(color(p.text_primary)),
                );
                crate::widgets::trailing(ui, |ui| {
                    let (rect, _) = ui.allocate_exact_size(
                        vec2(layout::CLOSE, layout::CLOSE),
                        egui::Sense::hover(),
                    );
                    if cr::close_button(ui, look, rect, Id::unique("dialog-close")) {
                        close = true;
                    }
                });
            });
            ui.add_space(12.0);
            let max_height = (ctx.content_rect().height() - 160.0).max(160.0);
            egui::ScrollArea::vertical()
                .id_salt("dialog-body")
                .max_height(max_height)
                .show(ui, |ui| {
                    body(ui, main, look, &mut dialog, actions, &mut close)
                });
        });
    if response.should_close() {
        close = true;
    }
    if close {
        actions.push(Action::CloseDialog);
    }
    // An action may replace the dialog this frame; it is applied after drawing.
    main.dialog = Some(dialog);
}

// --- Form pieces (also used by the task panel) ------------------------------------------------

pub fn label(ui: &mut Ui, look: &Look, text: &str) {
    ui.add_space(8.0);
    ui.label(
        RichText::new(text)
            .size(font_size::SMALL)
            .color(color(look.palette.text_secondary)),
    );
    ui.add_space(2.0);
}

pub fn field(
    ui: &mut Ui,
    look: &Look,
    text: &mut String,
    hint: &str,
    focus: bool,
) -> egui::Response {
    let response = cr::field_frame(look)
        .inner_margin(egui::Margin::symmetric(8, 0))
        .show(ui, |ui| {
            ui.add(
                TextEdit::singleline(text)
                    .frame(egui::Frame::NONE)
                    .hint_text(hint)
                    .align(egui::Align2::LEFT_CENTER)
                    .desired_width(ui.available_width()),
            )
        })
        .inner;
    if focus && ui.memory(|m| m.focused().is_none()) {
        response.request_focus();
    }
    response
}

pub fn area(
    ui: &mut Ui,
    look: &Look,
    text: &mut String,
    hint: &str,
    rows: usize,
) -> egui::Response {
    cr::field_frame(look)
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.add(
                TextEdit::multiline(text)
                    .frame(egui::Frame::NONE)
                    .hint_text(hint)
                    .desired_rows(rows)
                    .desired_width(ui.available_width()),
            )
        })
        .inner
}

/// A drop-down of `options`, each with its name. Returns whether the choice changed.
pub fn pick<T: Copy + PartialEq>(
    ui: &mut Ui,
    id: &str,
    value: &mut T,
    options: &[(T, String)],
) -> bool {
    let before = *value;
    let current = options
        .iter()
        .find(|(v, _)| *v == *value)
        .map(|(_, n)| n.clone())
        .unwrap_or_else(|| "…".to_string());
    egui::ComboBox::from_id_salt(id)
        .selected_text(current)
        .width(ui.available_width().min(260.0))
        .show_ui(ui, |ui| {
            for (v, name) in options {
                ui.selectable_value(value, *v, name.as_str());
            }
        });
    *value != before
}

pub fn statuses() -> Vec<(Status, String)> {
    Status::ALL
        .iter()
        .map(|s| (*s, status_name(*s).to_string()))
        .collect()
}

pub fn priorities() -> Vec<(Priority, String)> {
    Priority::ALL
        .iter()
        .map(|p| (*p, priority_name(*p).to_string()))
        .collect()
}

/// Who a task may be assigned to: nobody, or a member who is not a guest.
pub fn assignees(main: &Main) -> Vec<(Option<UserId>, String)> {
    let mut out = vec![(None, tr("Nobody").to_string())];
    for m in main.model.current_members() {
        if m.role != Role::Guest {
            out.push((Some(m.user.id), m.user.display_name.clone()));
        }
    }
    out
}

/// A project's milestones, by their opened names.
pub fn milestones(main: &Main, project: ProjectId) -> Vec<(Option<MilestoneId>, String)> {
    let mut out = vec![(None, tr("No milestone").to_string())];
    if let Some(board) = main.model.board(project) {
        for m in &board.milestones {
            let name = main
                .model
                .milestone_name(m.id)
                .unwrap_or_else(|| tr("Waiting for this project's keys"));
            out.push((Some(m.id), name.to_string()));
        }
    }
    out
}

fn buttons(ui: &mut Ui, look: &Look, confirm: &str, enabled: bool) -> (bool, bool) {
    ui.add_space(16.0);
    let mut out = (false, false);
    crate::widgets::trailing(ui, |ui| {
        out.0 = cr::primary_button(ui, look, confirm, enabled).clicked();
        out.1 = cr::secondary_button(ui, look, tr("Cancel"), true).clicked();
    });
    out
}

fn note(ui: &mut Ui, look: &Look, text: &str) {
    ui.label(
        RichText::new(text)
            .size(font_size::SMALL)
            .color(color(look.palette.text_secondary)),
    );
}

fn enter(ui: &Ui, response: &egui::Response) -> bool {
    response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
}

// --- The dialogs -------------------------------------------------------------------------------

fn body(
    ui: &mut Ui,
    main: &Main,
    look: &Look,
    dialog: &mut Dialog,
    actions: &mut Vec<Action>,
    close: &mut bool,
) {
    match dialog {
        Dialog::CreateWorkspace { name } => {
            note(
                ui,
                look,
                tr("A workspace holds your company's projects and people."),
            );
            label(ui, look, tr("Name"));
            let r = field(ui, look, name, tr("Acme Inc."), true);
            let ok = !name.trim().is_empty();
            let (go, cancel) = buttons(ui, look, tr("Create"), ok);
            if ok && (go || enter(ui, &r)) {
                actions.push(Action::CreateWorkspace(name.trim().to_string()));
                *close = true;
            }
            *close |= cancel;
        }
        Dialog::JoinWorkspace { code } => {
            note(
                ui,
                look,
                tr("Paste the invitation code someone in the workspace sent you."),
            );
            label(ui, look, tr("Invitation code"));
            let r = field(ui, look, code, "", true);
            let ok = !code.trim().is_empty();
            let (go, cancel) = buttons(ui, look, tr("Join"), ok);
            if ok && (go || enter(ui, &r)) {
                actions.push(Action::JoinWorkspace(code.trim().to_string()));
                *close = true;
            }
            *close |= cancel;
        }
        Dialog::Invite {
            invite,
            guest,
            uses,
            hours,
        } => invite_body(ui, look, invite, guest, uses, hours, actions, close),
        Dialog::NewProject {
            name,
            description,
            template,
        } => {
            label(ui, look, tr("Name"));
            let r = field(ui, look, name, tr("Website relaunch"), true);
            label(ui, look, tr("Description"));
            area(
                ui,
                look,
                description,
                tr("What this project is for (optional)"),
                2,
            );
            label(ui, look, tr("Start from"));
            egui::Grid::new("templates")
                .num_columns(2)
                .spacing(vec2(8.0, 8.0))
                .show(ui, |ui| {
                    for (i, kind) in Kind::ALL.iter().enumerate() {
                        template_card(ui, look, *kind, template);
                        if i % 2 == 1 {
                            ui.end_row();
                        }
                    }
                });
            note(
                ui,
                look,
                tr("The project's description, milestones and tasks are end-to-end encrypted."),
            );
            let ok = !name.trim().is_empty();
            let (go, cancel) = buttons(ui, look, tr("Create project"), ok);
            if ok && (go || enter(ui, &r)) {
                actions.push(Action::CreateProject {
                    name: name.trim().to_string(),
                    description: description.trim().to_string(),
                    template: *template,
                });
                *close = true;
            }
            *close |= cancel;
        }
        Dialog::NewTask {
            project,
            title,
            status,
            priority,
            assignee,
            milestone,
            due,
        } => {
            let editable: Vec<(ProjectId, String)> = main
                .model
                .current_projects()
                .iter()
                .filter(|p| main.can_edit(p.id))
                .map(|p| (p.id, p.name.clone()))
                .collect();
            label(ui, look, tr("Project"));
            if pick(ui, "new-task-project", project, &editable) {
                *milestone = None;
            }
            label(ui, look, tr("Title"));
            let r = field(ui, look, title, tr("What needs doing"), true);
            ui.columns(2, |cols| {
                label(&mut cols[0], look, tr("Status"));
                pick(&mut cols[0], "new-task-status", status, &statuses());
                label(&mut cols[1], look, tr("Priority"));
                pick(&mut cols[1], "new-task-priority", priority, &priorities());
            });
            label(ui, look, tr("Assignee"));
            pick(ui, "new-task-assignee", assignee, &assignees(main));
            label(ui, look, tr("Milestone"));
            pick(
                ui,
                "new-task-milestone",
                milestone,
                &milestones(main, *project),
            );
            label(ui, look, tr("Due date"));
            field(ui, look, due, "YYYY-MM-DD", false);
            let date = dates::parse_date(due);
            if date.is_none() {
                note(ui, look, tr("Write the date as YYYY-MM-DD."));
            }
            let ok = !title.trim().is_empty() && date.is_some();
            let (go, cancel) = buttons(ui, look, tr("Add task"), ok);
            if ok && (go || enter(ui, &r)) {
                actions.push(Action::CreateTask {
                    project: *project,
                    title: title.trim().to_string(),
                    status: *status,
                    priority: *priority,
                    assignee: *assignee,
                    milestone: *milestone,
                    due_at: date.flatten(),
                });
                *close = true;
            }
            *close |= cancel;
        }
        Dialog::Milestone {
            project,
            id,
            name,
            description,
            due,
        } => {
            label(ui, look, tr("Name"));
            let r = field(ui, look, name, tr("Beta ready"), true);
            label(ui, look, tr("Description"));
            area(
                ui,
                look,
                description,
                tr("What reaching it means (optional)"),
                2,
            );
            label(ui, look, tr("Due date"));
            field(ui, look, due, "YYYY-MM-DD", false);
            let date = dates::parse_date(due);
            if date.is_none() {
                note(ui, look, tr("Write the date as YYYY-MM-DD."));
            }
            let ok = !name.trim().is_empty() && date.is_some();
            let confirm = if id.is_some() {
                tr("Save")
            } else {
                tr("Add milestone")
            };
            let (go, cancel) = buttons(ui, look, confirm, ok);
            if ok && (go || enter(ui, &r)) {
                actions.push(Action::SaveMilestone {
                    project: *project,
                    id: *id,
                    name: name.trim().to_string(),
                    description: description.trim().to_string(),
                    due_at: date.flatten(),
                });
            }
            *close |= cancel;
        }
        Dialog::ProjectSettings {
            id,
            name,
            description,
            archived,
        } => project_settings(
            ui,
            main,
            look,
            *id,
            name,
            description,
            archived,
            actions,
            close,
        ),
        Dialog::Integrations(d) => integrations(ui, main, look, d, actions),
        Dialog::ConfirmDeleteTask { id } => {
            note(
                ui,
                look,
                tr("The task, its checklist and its comments are deleted for everyone. This cannot be undone."),
            );
            let (go, cancel) = buttons(ui, look, tr("Delete"), true);
            if go {
                actions.push(Action::DeleteTask(*id));
            }
            *close |= cancel;
        }
        Dialog::ConfirmDeleteMilestone { id } => {
            note(
                ui,
                look,
                tr("Its tasks stay in the project, without a milestone. This cannot be undone."),
            );
            let (go, cancel) = buttons(ui, look, tr("Delete"), true);
            if go {
                actions.push(Action::DeleteMilestone(*id));
            }
            *close |= cancel;
        }
    }
}

fn template_card(ui: &mut Ui, look: &Look, kind: Kind, selected: &mut Kind) {
    let p = &look.palette;
    let chosen = *selected == kind;
    let frame = egui::Frame::NONE
        .fill(color(if chosen {
            p.accent_subtle
        } else {
            p.control_bg
        }))
        .stroke(egui::Stroke::new(
            1.0,
            color(if chosen {
                p.accent_default
            } else {
                p.border_subtle
            }),
        ))
        .corner_radius(egui::CornerRadius::same(layout::CONTROL_RADIUS))
        .inner_margin(egui::Margin::same(10));
    let response = frame
        .show(ui, |ui| {
            ui.set_width(220.0);
            ui.horizontal(|ui| {
                crate::widgets::icon(ui, kind.icon(), 18.0, color(p.accent_text));
                ui.label(
                    RichText::new(kind.name())
                        .strong()
                        .color(color(p.text_primary)),
                );
            });
            ui.label(
                RichText::new(kind.summary())
                    .size(font_size::SMALL)
                    .color(color(p.text_secondary)),
            );
        })
        .response
        .interact(egui::Sense::click());
    if response.clicked() {
        *selected = kind;
    }
}

#[allow(clippy::too_many_arguments)]
fn invite_body(
    ui: &mut Ui,
    look: &Look,
    invite: &mut Option<Invite>,
    guest: &mut bool,
    uses: &mut u32,
    hours: &mut u32,
    actions: &mut Vec<Action>,
    close: &mut bool,
) {
    let p = &look.palette;
    if let Some(i) = invite {
        note(
            ui,
            look,
            tr("Send this code to the people you are inviting. It is shown only once."),
        );
        ui.add_space(8.0);
        ui.label(
            RichText::new(&i.code)
                .monospace()
                .size(font_size::LARGE)
                .color(color(p.text_primary)),
        );
        let role = if i.role == Role::Guest {
            tr("Guest")
        } else {
            tr("Member")
        };
        note(
            ui,
            look,
            &trf(
                "{} · up to {} people · until {}",
                &[
                    role,
                    &i.max_uses.to_string(),
                    &dates::date_text(i.expires_at),
                ],
            ),
        );
        ui.add_space(12.0);
        crate::widgets::trailing(ui, |ui| {
            if cr::primary_button(ui, look, tr("Copy code"), true).clicked() {
                actions.push(Action::Copy(i.code.clone()));
            }
            if cr::secondary_button(ui, look, tr("Done"), true).clicked() {
                *close = true;
            }
        });
        return;
    }
    label(ui, look, tr("They join as"));
    ui.horizontal(|ui| {
        ui.selectable_value(guest, false, tr("Member"));
        ui.selectable_value(guest, true, tr("Guest"));
    });
    note(
        ui,
        look,
        if *guest {
            tr("Guests see only the projects you add them to, and can comment there.")
        } else {
            tr("Members see every project and work on every task.")
        },
    );
    label(ui, look, tr("How many people may use it"));
    ui.add(egui::Slider::new(uses, 1..=100));
    label(ui, look, tr("Valid for"));
    ui.horizontal(|ui| {
        ui.selectable_value(hours, 24, tr("1 day"));
        ui.selectable_value(hours, 168, tr("7 days"));
        ui.selectable_value(hours, 720, tr("30 days"));
    });
    let (go, cancel) = buttons(ui, look, tr("Create invitation"), true);
    if go {
        actions.push(Action::CreateInvite {
            guest: *guest,
            hours: *hours,
            uses: *uses,
        });
    }
    *close |= cancel;
}

#[allow(clippy::too_many_arguments)]
fn project_settings(
    ui: &mut Ui,
    main: &Main,
    look: &Look,
    id: ProjectId,
    name: &mut String,
    description: &mut String,
    archived: &mut bool,
    actions: &mut Vec<Action>,
    close: &mut bool,
) {
    label(ui, look, tr("Name"));
    field(ui, look, name, "", false);
    label(ui, look, tr("Description"));
    area(
        ui,
        look,
        description,
        tr("What this project is for (optional)"),
        3,
    );
    ui.add_space(8.0);
    ui.checkbox(archived, tr("Archived (read-only for everyone)"));

    let guests: Vec<_> = main
        .model
        .current_members()
        .iter()
        .filter(|m| m.role == Role::Guest)
        .cloned()
        .collect();
    if !guests.is_empty() {
        label(ui, look, tr("Guests who can see this project"));
        let allowed = main
            .model
            .project(id)
            .map(|p| p.guests.clone())
            .unwrap_or_default();
        for g in guests {
            let mut on = allowed.contains(&g.user.id);
            if ui.checkbox(&mut on, &g.user.display_name).changed() {
                actions.push(Action::SetGuest {
                    project: id,
                    user: g.user.id,
                    allowed: on,
                });
            }
        }
    }
    let ok = !name.trim().is_empty();
    let (go, cancel) = buttons(ui, look, tr("Save"), ok);
    if ok && go {
        actions.push(Action::SaveProject {
            id,
            name: name.trim().to_string(),
            description: description.trim().to_string(),
            archived: *archived,
        });
        *close = true;
    }
    *close |= cancel;
}

pub fn event_name(kind: ActivityKind) -> &'static str {
    match kind {
        ActivityKind::ProjectCreated => tr("Project created"),
        ActivityKind::ProjectUpdated => tr("Project changed"),
        ActivityKind::ProjectArchived => tr("Project archived"),
        ActivityKind::MilestoneCreated => tr("Milestone added"),
        ActivityKind::MilestoneUpdated => tr("Milestone changed"),
        ActivityKind::MilestoneDeleted => tr("Milestone deleted"),
        ActivityKind::MilestoneReached => tr("Milestone reached"),
        ActivityKind::TaskCreated => tr("Task added"),
        ActivityKind::TaskUpdated => tr("Task changed"),
        ActivityKind::TaskMoved => tr("Task moved"),
        ActivityKind::TaskAssigned => tr("Task assigned"),
        ActivityKind::TaskCompleted => tr("Task completed"),
        ActivityKind::TaskReopened => tr("Task reopened"),
        ActivityKind::TaskDeleted => tr("Task deleted"),
        ActivityKind::CommentAdded => tr("Comment added"),
    }
}

fn integrations(
    ui: &mut Ui,
    main: &Main,
    look: &Look,
    d: &mut Integrations,
    actions: &mut Vec<Action>,
) {
    let p = &look.palette;
    if let Some(secret) = &d.secret {
        egui::Frame::NONE
            .fill(color(p.status_warning_bg))
            .corner_radius(egui::CornerRadius::same(layout::CONTROL_RADIUS))
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| {
                ui.label(
                    RichText::new(tr("Copy it now: it is shown only once."))
                        .strong()
                        .color(color(p.status_warning_fg)),
                );
                ui.add(
                    egui::Label::new(
                        RichText::new(secret)
                            .monospace()
                            .color(color(p.text_primary)),
                    )
                    .wrap(),
                );
                if cr::secondary_button(ui, look, tr("Copy"), true).clicked() {
                    actions.push(Action::Copy(secret.clone()));
                }
            });
        ui.add_space(8.0);
    }

    ui.label(
        RichText::new(tr("API tokens"))
            .strong()
            .color(color(p.text_primary)),
    );
    note(
        ui,
        look,
        tr("A token lets another app (Ágora, a script) act as you: it sees and changes what you can. Revoke it when it is no longer needed."),
    );
    for t in main.model.tokens.clone() {
        ui.horizontal(|ui| {
            ui.label(RichText::new(&t.name).color(color(p.text_primary)));
            let used = t
                .last_used_at
                .map(|at| trf("used {}", &[&dates::when(at)]))
                .unwrap_or_else(|| tr("never used").to_string());
            note(ui, look, &used);
            crate::widgets::trailing(ui, |ui| {
                if cr::secondary_button(ui, look, tr("Revoke"), true).clicked() {
                    actions.push(Action::RevokeToken(t.id));
                }
            });
        });
    }
    ui.horizontal(|ui| {
        let width = ui.available_width() - 110.0;
        ui.allocate_ui(vec2(width.max(80.0), 32.0), |ui| {
            field(
                ui,
                look,
                &mut d.token_name,
                tr("Token name, e.g. Ágora"),
                false,
            )
        });
        let ok = !d.token_name.trim().is_empty();
        if cr::primary_button(ui, look, tr("Create token"), ok).clicked() && ok {
            actions.push(Action::CreateToken(d.token_name.trim().to_string()));
        }
    });

    let admin = main
        .model
        .current_workspace()
        .is_some_and(|w| w.role.can_manage_integrations());
    ui.add_space(16.0);
    ui.label(
        RichText::new(tr("Webhooks"))
            .strong()
            .color(color(p.text_primary)),
    );
    if !admin {
        note(
            ui,
            look,
            tr("Only the workspace's owners and admins manage webhooks."),
        );
        return;
    }
    note(
        ui,
        look,
        tr("Hito posts each change to the address, signed with a secret. In Ágora, add an incoming Hito webhook to a channel and paste its address here."),
    );
    for w in main.model.webhooks.clone() {
        ui.horizontal(|ui| {
            let ink = match w.last_delivery_ok {
                Some(true) => p.status_success_fg,
                Some(false) => p.status_danger_fg,
                None => p.text_tertiary,
            };
            crate::widgets::icon(ui, "link", 16.0, color(ink));
            ui.vertical(|ui| {
                ui.label(RichText::new(&w.name).color(color(p.text_primary)));
                ui.add(
                    egui::Label::new(
                        RichText::new(&w.url)
                            .size(font_size::SMALL)
                            .color(color(p.text_secondary)),
                    )
                    .truncate(),
                );
            });
            crate::widgets::trailing(ui, |ui| {
                if cr::secondary_button(ui, look, tr("Delete"), true).clicked() {
                    actions.push(Action::DeleteWebhook(w.id));
                }
            });
        });
    }
    label(ui, look, tr("Name"));
    field(ui, look, &mut d.hook_name, tr("Ágora #product"), false);
    label(ui, look, tr("Address"));
    field(ui, look, &mut d.hook_url, "https://", false);
    label(ui, look, tr("Signing key (optional)"));
    field(ui, look, &mut d.hook_secret, "", false);
    label(ui, look, tr("Project"));
    let mut projects = vec![(None, tr("Every project").to_string())];
    projects.extend(
        main.model
            .current_projects()
            .iter()
            .map(|p| (Some(p.id), p.name.clone())),
    );
    pick(ui, "hook-project", &mut d.hook_project, &projects);
    label(ui, look, tr("Send"));
    let mut all = d.hook_events.is_empty();
    if ui.checkbox(&mut all, tr("Everything")).changed() {
        d.hook_events = if all {
            Vec::new()
        } else {
            vec![ActivityKind::TaskCompleted, ActivityKind::MilestoneReached]
        };
    }
    if !all {
        egui::Grid::new("hook-events")
            .num_columns(2)
            .show(ui, |ui| {
                for (i, kind) in ActivityKind::ALL.iter().enumerate() {
                    let mut on = d.hook_events.contains(kind);
                    if ui.checkbox(&mut on, event_name(*kind)).changed() {
                        if on {
                            d.hook_events.push(*kind);
                        } else {
                            d.hook_events.retain(|k| k != kind);
                        }
                    }
                    if i % 2 == 1 {
                        ui.end_row();
                    }
                }
            });
    }
    let ok = !d.hook_name.trim().is_empty() && d.hook_url.trim().starts_with("http");
    ui.add_space(8.0);
    crate::widgets::trailing(ui, |ui| {
        if cr::primary_button(ui, look, tr("Add webhook"), ok).clicked() && ok {
            actions.push(Action::CreateWebhook {
                name: d.hook_name.trim().to_string(),
                url: d.hook_url.trim().to_string(),
                project: d.hook_project,
                events: d.hook_events.clone(),
                secret: d.hook_secret.trim().to_string(),
            });
        }
    });
}
