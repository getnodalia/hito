//! The centre of the window: the Overview, My tasks, and a project with its four views (Board,
//! List, Milestones, Activity).

use egui::{
    vec2, Align, Color32, CornerRadius, Layout, Margin, Rect, RichText, Sense, Stroke, Ui,
    UiBuilder,
};
use hito_core::{Priority, Progress, ProjectId, Status};
use hito_protocol::{Activity, ActivityKind, Project, Task};
use nodalia_theme::creator::{self as cr, layout, Look};
use nodalia_theme::font_size;
use nodalia_theme::for_egui::color;

use crate::app::{Action, Main};
use crate::dates;
use crate::dialogs::Dialog;
use crate::i18n::{tr, trf};
use crate::model::{Page, Tab};
use crate::view::empty_state;
use crate::widgets::{avatar, chip, icon, icon_button, priority_ink, progress_bar, status_colors};

pub fn show(ui: &mut Ui, main: &mut Main, look: &Look, _now: f64, actions: &mut Vec<Action>) {
    let full = ui.max_rect().shrink2(vec2(20.0, 16.0));
    ui.scope_builder(UiBuilder::new().max_rect(full), |ui| {
        if main.model.current_workspace().is_none() {
            no_workspace(ui, look, actions);
            return;
        }
        match main.page {
            Page::Overview => overview(ui, main, look, actions),
            Page::MyTasks => my_tasks(ui, main, look, actions),
            Page::Project(id) => project(ui, main, look, id, actions),
        }
    });
}

// --- Names -------------------------------------------------------------------------------------

pub fn status_name(status: Status) -> &'static str {
    match status {
        Status::Todo => tr("To do"),
        Status::InProgress => tr("In progress"),
        Status::InReview => tr("In review"),
        Status::Done => tr("Done"),
    }
}

pub fn priority_name(priority: Priority) -> &'static str {
    match priority {
        Priority::Low => tr("Low"),
        Priority::Normal => tr("Normal"),
        Priority::High => tr("High"),
        Priority::Urgent => tr("Urgent"),
    }
}

/// One line of the activity log: who did what.
pub fn activity_line(kind: ActivityKind, who: &str) -> String {
    match kind {
        ActivityKind::ProjectCreated => trf("{} created the project", &[who]),
        ActivityKind::ProjectUpdated => trf("{} changed the project", &[who]),
        ActivityKind::ProjectArchived => trf("{} archived the project", &[who]),
        ActivityKind::MilestoneCreated => trf("{} added a milestone", &[who]),
        ActivityKind::MilestoneUpdated => trf("{} changed a milestone", &[who]),
        ActivityKind::MilestoneDeleted => trf("{} deleted a milestone", &[who]),
        ActivityKind::MilestoneReached => tr("A milestone was reached").to_string(),
        ActivityKind::TaskCreated => trf("{} added a task", &[who]),
        ActivityKind::TaskUpdated => trf("{} changed a task", &[who]),
        ActivityKind::TaskMoved => trf("{} moved a task", &[who]),
        ActivityKind::TaskAssigned => trf("{} assigned a task", &[who]),
        ActivityKind::TaskCompleted => trf("{} completed a task", &[who]),
        ActivityKind::TaskReopened => trf("{} reopened a task", &[who]),
        ActivityKind::TaskDeleted => trf("{} deleted a task", &[who]),
        ActivityKind::CommentAdded => trf("{} commented on a task", &[who]),
    }
}

/// The text shown for a task this device cannot open yet.
pub fn sealed_title() -> &'static str {
    tr("Waiting for this project's keys")
}

/// Whether a task matches the filter typed in the top bar (title or a label, any case).
pub fn matches(main: &Main, task: &Task) -> bool {
    let query = main.search.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }
    main.model.task_text.get(&task.id).is_some_and(|c| {
        c.title.to_lowercase().contains(&query)
            || c.labels.iter().any(|l| l.to_lowercase().contains(&query))
    })
}

fn heading(ui: &mut Ui, look: &Look, text: &str) {
    ui.label(
        RichText::new(text)
            .size(font_size::HEADLINE)
            .strong()
            .color(color(look.palette.text_primary)),
    );
}

fn subheading(ui: &mut Ui, look: &Look, text: &str) {
    ui.add_space(14.0);
    ui.label(
        RichText::new(text)
            .size(font_size::LARGE)
            .strong()
            .color(color(look.palette.text_primary)),
    );
    ui.add_space(6.0);
}

fn muted(ui: &mut Ui, look: &Look, text: &str) {
    ui.label(
        RichText::new(text)
            .size(font_size::SMALL)
            .color(color(look.palette.text_secondary)),
    );
}

fn no_workspace(ui: &mut Ui, look: &Look, actions: &mut Vec<Action>) {
    empty_state(
        ui,
        look,
        "users",
        tr("Create a workspace for your company, or join one with an invitation code."),
    );
    ui.add_space(16.0);
    ui.vertical_centered(|ui| {
        ui.horizontal(|ui| {
            ui.add_space((ui.available_width() - 330.0).max(0.0) / 2.0);
            if cr::primary_button(ui, look, tr("Create a workspace"), true).clicked() {
                actions.push(Action::Dialog(Dialog::CreateWorkspace {
                    name: String::new(),
                }));
            }
            if cr::secondary_button(ui, look, tr("Join a workspace"), true).clicked() {
                actions.push(Action::Dialog(Dialog::JoinWorkspace {
                    code: String::new(),
                }));
            }
        });
    });
}

// --- Overview ------------------------------------------------------------------------------------

fn overview(ui: &mut Ui, main: &Main, look: &Look, actions: &mut Vec<Action>) {
    let p = &look.palette;
    let model = &main.model;
    let projects: Vec<&Project> = model
        .current_projects()
        .iter()
        .filter(|p| !p.archived)
        .collect();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            heading(ui, look, &trf("Hello, {}", &[&model.me.display_name]));
            muted(
                ui,
                look,
                model.current_workspace().map_or("", |w| w.name.as_str()),
            );
            ui.add_space(14.0);

            let mine = model.open_my_tasks();
            let now = dates::now_ms();
            let overdue_mine = mine
                .iter()
                .filter(|r| hito_core::task::overdue(r.task.status, r.task.due_at, now))
                .count();
            let overdue_all: u32 = projects.iter().map(|p| p.overdue).sum();
            let total = projects.iter().fold(Progress::default(), |acc, p| {
                let pr = model.progress(p);
                Progress {
                    total: acc.total.saturating_add(pr.total),
                    done: acc.done.saturating_add(pr.done),
                }
            });
            ui.horizontal_wrapped(|ui| {
                tile(
                    ui,
                    look,
                    "check-square",
                    &mine.len().to_string(),
                    tr("Open tasks for you"),
                );
                tile(
                    ui,
                    look,
                    "warning",
                    &overdue_mine.to_string(),
                    tr("Of yours, overdue"),
                );
                tile(
                    ui,
                    look,
                    "folder",
                    &projects.len().to_string(),
                    tr("Active projects"),
                );
                tile(
                    ui,
                    look,
                    "flag",
                    &overdue_all.to_string(),
                    tr("Overdue in the workspace"),
                );
                tile(
                    ui,
                    look,
                    "chart-bar",
                    &format!("{} %", total.percent()),
                    tr("Done across projects"),
                );
            });

            subheading(ui, look, tr("Projects"));
            if projects.is_empty() {
                muted(ui, look, tr("No projects yet."));
                if model
                    .current_workspace()
                    .is_some_and(|w| w.role.can_create_project())
                    && cr::primary_button(ui, look, tr("New project"), true).clicked()
                {
                    actions.push(Action::Dialog(Dialog::new_project()));
                }
            }
            for project in &projects {
                let progress = model.progress(project);
                let response = cr::island(look)
                    .fill(color(p.surface_window))
                    .inner_margin(Margin::symmetric(14, 10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            icon(ui, "folder", 16.0, color(p.accent_text));
                            ui.label(
                                RichText::new(&project.name)
                                    .strong()
                                    .color(color(p.text_primary)),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if project.overdue > 0 {
                                    let (fg, bg) = (p.status_danger_fg, p.status_danger_bg);
                                    chip(
                                        ui,
                                        &trf("{} overdue", &[&project.overdue.to_string()]),
                                        color(fg),
                                        color(bg),
                                    );
                                }
                                muted(
                                    ui,
                                    look,
                                    &trf(
                                        "{} of {} done",
                                        &[&progress.done.to_string(), &progress.total.to_string()],
                                    ),
                                );
                            });
                        });
                        progress_bar(ui, look, progress, 0.0, true);
                    })
                    .response
                    .interact(Sense::click());
                if response.clicked() {
                    actions.push(Action::ShowPage(Page::Project(project.id)));
                }
                ui.add_space(6.0);
            }

            subheading(ui, look, tr("Due soon"));
            let soon: Vec<_> = mine
                .iter()
                .filter(|r| r.task.due_at.is_some_and(|d| dates::days_until(d) <= 7))
                .take(8)
                .collect();
            if soon.is_empty() {
                muted(
                    ui,
                    look,
                    tr("Nothing of yours is due in the next seven days."),
                );
            }
            for r in soon {
                task_row(ui, main, look, &r.task, Some(&r.project_name), actions);
            }
        });
}

fn tile(ui: &mut Ui, look: &Look, glyph: &str, value: &str, label: &str) {
    let p = &look.palette;
    cr::island(look)
        .fill(color(p.surface_window))
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(150.0);
            ui.horizontal(|ui| {
                icon(ui, glyph, 16.0, color(p.accent_text));
                ui.label(
                    RichText::new(value)
                        .size(font_size::TITLE)
                        .strong()
                        .color(color(p.text_primary)),
                );
            });
            muted(ui, look, label);
        });
}

// --- My tasks ------------------------------------------------------------------------------------

fn my_tasks(ui: &mut Ui, main: &Main, look: &Look, actions: &mut Vec<Action>) {
    heading(ui, look, tr("My tasks"));
    muted(
        ui,
        look,
        tr("Everything assigned to you, in every project."),
    );
    ui.add_space(8.0);
    if !main.model.my_tasks_loaded {
        ui.spinner();
        return;
    }
    let list: Vec<_> = main
        .model
        .open_my_tasks()
        .into_iter()
        .filter(|r| matches(main, &r.task))
        .collect();
    if list.is_empty() {
        empty_state(
            ui,
            look,
            "success",
            tr("Nothing assigned to you is open. Well done."),
        );
        return;
    }
    let now = dates::now_ms();
    let group = |t: &Task| -> usize {
        match t.due_at {
            Some(d) if hito_core::task::overdue(t.status, Some(d), now) => 0,
            Some(d) if dates::days_until(d) <= 0 => 1,
            Some(d) if dates::days_until(d) <= 7 => 2,
            Some(_) => 3,
            None => 4,
        }
    };
    let titles = [
        tr("Overdue"),
        tr("Today"),
        tr("This week"),
        tr("Later"),
        tr("No date"),
    ];
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (i, title) in titles.iter().enumerate() {
                let rows: Vec<_> = list.iter().filter(|r| group(&r.task) == i).collect();
                if rows.is_empty() {
                    continue;
                }
                subheading(ui, look, title);
                for r in rows {
                    task_row(ui, main, look, &r.task, Some(&r.project_name), actions);
                }
            }
        });
}

/// One task as a row: a done checkbox, the title, its project, the due date and the assignee.
fn task_row(
    ui: &mut Ui,
    main: &Main,
    look: &Look,
    task: &Task,
    project: Option<&str>,
    actions: &mut Vec<Action>,
) {
    let p = &look.palette;
    let selected = main.task == Some(task.id);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
    let fill = if selected {
        color(p.accent_subtle)
    } else if response.hovered() {
        color(p.control_bg_hover)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter()
        .rect_filled(rect, CornerRadius::same(layout::CONTROL_RADIUS), fill);
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink2(vec2(8.0, 0.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    let ui = &mut child;
    let mut done = task.status.is_done();
    let editable = main.can_edit(task.project_id);
    if ui
        .add_enabled(editable, egui::Checkbox::without_text(&mut done))
        .on_hover_text(tr("Mark as done"))
        .changed()
    {
        actions.push(Action::MoveTask {
            id: task.id,
            status: if done { Status::Done } else { Status::Todo },
        });
    }
    let title = main.model.title(task.id).unwrap_or(sealed_title());
    let mut text = RichText::new(title).color(color(if done {
        p.text_tertiary
    } else {
        p.text_primary
    }));
    if done {
        text = text.strikethrough();
    }
    ui.add(egui::Label::new(text).truncate().selectable(false));
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if let Some(user) = task.assignee_id {
            let name = main.model.name_of(user).unwrap_or("?");
            avatar(ui, look, user.0, name, 22.0, None);
        }
        due_chip(ui, look, task);
        let (fg, bg) = status_colors(look, task.status);
        chip(ui, status_name(task.status), fg, bg);
        if let Some(project) = project {
            muted(ui, look, project);
        }
    });
    if response.clicked() {
        actions.push(Action::OpenTask(task.id));
    }
}

fn due_chip(ui: &mut Ui, look: &Look, task: &Task) {
    let p = &look.palette;
    let Some(due) = task.due_at else {
        return;
    };
    let late = hito_core::task::overdue(task.status, Some(due), dates::now_ms());
    let (fg, bg) = if late {
        (color(p.status_danger_fg), color(p.status_danger_bg))
    } else {
        (color(p.text_secondary), color(p.control_bg_hover))
    };
    chip(ui, &dates::short(due), fg, bg);
}

// --- A project -------------------------------------------------------------------------------------

fn project(ui: &mut Ui, main: &mut Main, look: &Look, id: ProjectId, actions: &mut Vec<Action>) {
    let p = &look.palette;
    let Some(project) = main.model.project(id).cloned() else {
        ui.spinner();
        return;
    };
    let progress = main.model.progress(&project);
    ui.horizontal(|ui| {
        heading(ui, look, &project.name);
        if project.archived {
            chip(
                ui,
                tr("Archived"),
                color(p.text_secondary),
                color(p.control_bg_hover),
            );
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if main
                .role_in(id)
                .is_some_and(|r| r != hito_core::Role::Guest)
                && icon_button(ui, look, "settings", tr("Project settings"), false).clicked()
            {
                let description = main
                    .model
                    .description_text
                    .get(&id)
                    .cloned()
                    .unwrap_or_default();
                actions.push(Action::Dialog(Dialog::ProjectSettings {
                    id,
                    name: project.name.clone(),
                    description,
                    archived: project.archived,
                }));
            }
        });
    });
    if let Some(text) = main.model.description_text.get(&id) {
        if !text.is_empty() {
            ui.add(egui::Label::new(RichText::new(text).color(color(p.text_secondary))).truncate());
        }
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        progress_bar(ui, look, progress, 260.0, true);
        ui.add_space(8.0);
        muted(
            ui,
            look,
            &trf(
                "{} of {} tasks done",
                &[&progress.done.to_string(), &progress.total.to_string()],
            ),
        );
    });
    if main.model.waiting_for_keys(id) {
        ui.add_space(6.0);
        egui::Frame::NONE
            .fill(color(p.status_warning_bg))
            .corner_radius(CornerRadius::same(layout::CONTROL_RADIUS))
            .inner_margin(Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    icon(ui, "key", 16.0, color(p.status_warning_fg));
                    ui.label(
                        RichText::new(tr("This device does not have this project's keys yet. They arrive when a teammate with access opens Hito."))
                            .color(color(p.status_warning_fg)),
                    );
                });
            });
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        for (tab, label) in [
            (Tab::Board, tr("Board")),
            (Tab::List, tr("List")),
            (Tab::Milestones, tr("Milestones")),
            (Tab::Activity, tr("Activity")),
        ] {
            if cr::tab(ui, look, label, main.tab == tab).clicked() {
                actions.push(Action::ShowTab(tab));
            }
        }
    });
    ui.add_space(8.0);
    let Some(board) = main.model.board(id) else {
        ui.spinner();
        return;
    };
    let _ = board;
    match main.tab {
        Tab::Board => board_view(ui, main, look, id, actions),
        Tab::List => list_view(ui, main, look, id, actions),
        Tab::Milestones => milestones_view(ui, main, look, id, actions),
        Tab::Activity => activity_view(ui, main, look, id, actions),
    }
}

// --- Board -----------------------------------------------------------------------------------------

fn board_view(ui: &mut Ui, main: &mut Main, look: &Look, id: ProjectId, actions: &mut Vec<Action>) {
    let p = &look.palette;
    let editable = main.can_edit(id);
    let area = ui.available_rect_before_wrap();
    let gap = 10.0;
    let width = ((area.width() - gap * 3.0) / 4.0).max(200.0);
    let pointer = ui.ctx().pointer_interact_pos();
    let released = ui.input(|i| i.pointer.any_released());
    let mut drop_target: Option<Status> = None;

    egui::ScrollArea::horizontal()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for status in Status::ALL {
                    let column_rect =
                        Rect::from_min_size(ui.cursor().min, vec2(width, area.height()));
                    let hovered =
                        main.drag.is_some() && pointer.is_some_and(|pos| column_rect.contains(pos));
                    if hovered {
                        drop_target = Some(status);
                    }
                    let fill = if hovered {
                        color(p.accent_subtle)
                    } else {
                        color(p.surface_window)
                    };
                    ui.scope_builder(UiBuilder::new().max_rect(column_rect), |ui| {
                        egui::Frame::NONE
                            .fill(fill)
                            .corner_radius(CornerRadius::same(12))
                            .inner_margin(Margin::same(8))
                            .show(ui, |ui| {
                                ui.set_min_size(column_rect.size() - vec2(16.0, 16.0));
                                ui.set_max_width(width - 16.0);
                                column(ui, main, look, id, status, editable, actions);
                            });
                    });
                    ui.allocate_space(vec2(width, 0.0));
                }
            });
        });

    if let Some(dragged) = main.drag {
        if released {
            if let Some(status) = drop_target {
                actions.push(Action::MoveTask {
                    id: dragged,
                    status,
                });
            }
            main.drag = None;
        } else if let Some(pos) = pointer {
            // The card follows the pointer.
            let title = main
                .model
                .title(dragged)
                .unwrap_or(sealed_title())
                .to_string();
            let painter = ui.ctx().layer_painter(egui::LayerId::new(
                egui::Order::Tooltip,
                egui::Id::unique("hito-drag"),
            ));
            let rect = Rect::from_min_size(pos + vec2(-20.0, -14.0), vec2(width - 24.0, 40.0));
            painter.rect_filled(rect, CornerRadius::same(10), color(p.surface_panel));
            painter.rect_stroke(
                rect,
                CornerRadius::same(10),
                Stroke::new(1.5, color(p.accent_default)),
                egui::StrokeKind::Inside,
            );
            painter.text(
                rect.left_center() + vec2(10.0, 0.0),
                egui::Align2::LEFT_CENTER,
                title,
                egui::FontId::proportional(font_size::BODY),
                color(p.text_primary),
            );
        }
    }
}

fn column(
    ui: &mut Ui,
    main: &mut Main,
    look: &Look,
    id: ProjectId,
    status: Status,
    editable: bool,
    actions: &mut Vec<Action>,
) {
    let tasks: Vec<Task> = main
        .model
        .column(id, status)
        .into_iter()
        .filter(|t| matches(main, t))
        .cloned()
        .collect();
    ui.horizontal(|ui| {
        let (fg, bg) = status_colors(look, status);
        chip(ui, status_name(status), fg, bg);
        muted(ui, look, &tasks.len().to_string());
    });
    ui.add_space(6.0);
    if editable {
        let text = main.quick_add.entry((id, status)).or_default();
        let response = cr::field_frame(look)
            .inner_margin(Margin::symmetric(8, 0))
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(text)
                        .frame(egui::Frame::NONE)
                        .align(egui::Align2::LEFT_CENTER)
                        .hint_text(tr("Add a task"))
                        .desired_width(ui.available_width()),
                )
            })
            .inner;
        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            actions.push(Action::QuickAdd(id, status));
            response.request_focus();
        }
        ui.add_space(6.0);
    }
    egui::ScrollArea::vertical()
        .id_salt(("column", status.as_str()))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for task in &tasks {
                card(ui, main, look, task, editable, actions);
                ui.add_space(6.0);
            }
        });
}

fn card(
    ui: &mut Ui,
    main: &mut Main,
    look: &Look,
    task: &Task,
    editable: bool,
    actions: &mut Vec<Action>,
) {
    let p = &look.palette;
    let selected = main.task == Some(task.id);
    let dragging = main.drag == Some(task.id);
    let content = main.model.task_text.get(&task.id);
    let frame = egui::Frame::NONE
        .fill(color(p.surface_panel))
        .corner_radius(CornerRadius::same(10))
        .stroke(Stroke::new(
            if selected { 1.5 } else { 1.0 },
            color(if selected {
                p.accent_default
            } else {
                p.border_subtle
            }),
        ))
        .inner_margin(Margin::same(10));
    let inner = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        if dragging {
            ui.set_opacity(0.4);
        }
        let title = content.map_or(sealed_title(), |c| c.title.as_str());
        ui.add(
            egui::Label::new(RichText::new(title).color(color(if content.is_some() {
                p.text_primary
            } else {
                p.text_tertiary
            })))
            .wrap()
            .selectable(false),
        );
        if let Some(c) = content {
            if !c.labels.is_empty() {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    for l in &c.labels {
                        chip(ui, l, color(p.accent_text), color(p.accent_subtle));
                    }
                });
            }
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if matches!(task.priority, Priority::High | Priority::Urgent) {
                chip(
                    ui,
                    priority_name(task.priority),
                    priority_ink(look, task.priority),
                    color(p.control_bg_hover),
                );
            }
            due_chip(ui, look, task);
            if let Some(c) = content {
                if !c.checklist.is_empty() {
                    let done = c.checklist.iter().filter(|i| i.done).count();
                    icon(ui, "check", 12.0, color(p.text_secondary));
                    muted(ui, look, &format!("{done}/{}", c.checklist.len()));
                }
            }
            if task.comment_count > 0 {
                icon(ui, "comment", 12.0, color(p.text_secondary));
                muted(ui, look, &task.comment_count.to_string());
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(user) = task.assignee_id {
                    let name = main.model.name_of(user).unwrap_or("?");
                    avatar(ui, look, user.0, name, 20.0, None);
                }
            });
        });
    });
    let sense = if editable {
        Sense::click_and_drag()
    } else {
        Sense::click()
    };
    let response = ui.interact(
        inner.response.rect,
        ui.make_persistent_id(("card", task.id.0)),
        sense,
    );
    if response.drag_started() {
        main.drag = Some(task.id);
    }
    if response.clicked() {
        actions.push(Action::OpenTask(task.id));
    }
    if response.hovered() && main.drag.is_none() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
}

// --- List ------------------------------------------------------------------------------------------

fn list_view(ui: &mut Ui, main: &Main, look: &Look, id: ProjectId, actions: &mut Vec<Action>) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let mut any = false;
            for status in Status::ALL {
                let tasks: Vec<&Task> = main
                    .model
                    .column(id, status)
                    .into_iter()
                    .filter(|t| matches(main, t))
                    .collect();
                if tasks.is_empty() {
                    continue;
                }
                any = true;
                subheading(
                    ui,
                    look,
                    &format!("{} · {}", status_name(status), tasks.len()),
                );
                for task in tasks {
                    task_row(ui, main, look, task, None, actions);
                }
            }
            if !any {
                empty_state(ui, look, "list", tr("No tasks here yet."));
            }
        });
}

// --- Milestones ------------------------------------------------------------------------------------

fn milestones_view(
    ui: &mut Ui,
    main: &Main,
    look: &Look,
    id: ProjectId,
    actions: &mut Vec<Action>,
) {
    let p = &look.palette;
    let editable = main.can_edit(id);
    if editable && cr::primary_button(ui, look, tr("New milestone"), true).clicked() {
        actions.push(Action::Dialog(Dialog::milestone(id, None, "", "", None)));
    }
    ui.add_space(8.0);
    let Some(board) = main.model.board(id) else {
        return;
    };
    if board.milestones.is_empty() {
        empty_state(
            ui,
            look,
            "flag",
            tr("Milestones mark the points that matter: a delivery, a launch, a review."),
        );
        return;
    }
    let now = dates::now_ms();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for m in &board.milestones {
                let progress = main.model.milestone_progress(m);
                let content = main.model.milestone_text.get(&m.id);
                let name = content.map_or(sealed_title(), |c| c.name.as_str());
                cr::island(look)
                    .fill(color(p.surface_window))
                    .inner_margin(Margin::symmetric(14, 10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            let reached = progress.complete();
                            icon(
                                ui,
                                if reached { "success" } else { "flag" },
                                18.0,
                                color(if reached {
                                    p.status_success_fg
                                } else {
                                    p.accent_text
                                }),
                            );
                            ui.label(
                                RichText::new(name)
                                    .strong()
                                    .size(font_size::LARGE)
                                    .color(color(p.text_primary)),
                            );
                            if let Some(due) = m.due_at {
                                let late = !progress.complete() && due < now;
                                let (fg, bg) = if late {
                                    (p.status_danger_fg, p.status_danger_bg)
                                } else {
                                    (p.text_secondary, p.control_bg_hover)
                                };
                                chip(ui, &dates::short(due), color(fg), color(bg));
                            }
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if editable {
                                    if icon_button(
                                        ui,
                                        look,
                                        "delete",
                                        tr("Delete milestone"),
                                        false,
                                    )
                                    .clicked()
                                    {
                                        actions.push(Action::Dialog(
                                            Dialog::ConfirmDeleteMilestone { id: m.id },
                                        ));
                                    }
                                    if content.is_some()
                                        && icon_button(
                                            ui,
                                            look,
                                            "edit",
                                            tr("Edit milestone"),
                                            false,
                                        )
                                        .clicked()
                                    {
                                        let c = content.cloned().unwrap_or_default();
                                        actions.push(Action::Dialog(Dialog::milestone(
                                            id,
                                            Some(m.id),
                                            &c.name,
                                            &c.description,
                                            m.due_at,
                                        )));
                                    }
                                }
                            });
                        });
                        if let Some(c) = content {
                            if !c.description.is_empty() {
                                ui.label(
                                    RichText::new(&c.description).color(color(p.text_secondary)),
                                );
                            }
                        }
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            progress_bar(ui, look, progress, 260.0, true);
                            muted(
                                ui,
                                look,
                                &trf(
                                    "{} of {} tasks done",
                                    &[&progress.done.to_string(), &progress.total.to_string()],
                                ),
                            );
                        });
                    });
                ui.add_space(8.0);
            }
        });
}

// --- Activity --------------------------------------------------------------------------------------

fn activity_view(ui: &mut Ui, main: &Main, look: &Look, id: ProjectId, actions: &mut Vec<Action>) {
    let Some(board) = main.model.board(id) else {
        return;
    };
    if board.activity.is_empty() {
        empty_state(ui, look, "history", tr("Nothing has happened here yet."));
        return;
    }
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for a in &board.activity {
                activity_row(ui, main, look, a, actions);
            }
            if board.more_activity {
                ui.add_space(8.0);
                if cr::secondary_button(ui, look, tr("Show older activity"), true).clicked() {
                    actions.push(Action::OlderActivity(id));
                }
            }
        });
}

fn activity_row(ui: &mut Ui, main: &Main, look: &Look, a: &Activity, actions: &mut Vec<Action>) {
    let p = &look.palette;
    ui.horizontal(|ui| {
        let who = a
            .actor
            .as_ref()
            .map_or(tr("Someone"), |u| u.display_name.as_str());
        let seed = a.actor.as_ref().map_or(0, |u| u.id.0);
        avatar(ui, look, seed, who, 28.0, None);
        ui.add_space(6.0);
        ui.vertical(|ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(activity_line(a.kind, who)).color(color(p.text_primary)));
                if let Some(subject) = main.model.subject_text.get(&a.id) {
                    let task = a.task_id.filter(|t| main.model.task(*t).is_some());
                    let text = RichText::new(subject).strong().color(color(p.accent_text));
                    match task {
                        Some(t) => {
                            if ui.link(text).clicked() {
                                actions.push(Action::OpenTask(t));
                            }
                        }
                        None => {
                            ui.label(text);
                        }
                    }
                }
            });
            muted(ui, look, &dates::when(a.at));
        });
    });
    ui.add_space(8.0);
}
