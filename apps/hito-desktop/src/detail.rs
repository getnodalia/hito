//! The task panel: every field of the open task, saved together, its checklist and its
//! comments. A task this device cannot open yet is shown read-only until the keys arrive.

use egui::{RichText, Ui};
use hito_core::roles::can_delete_comment;
use hito_protocol::ChecklistItem;
use nodalia_theme::creator::{self as cr, Look};
use nodalia_theme::font_size;
use nodalia_theme::for_egui::color;

use crate::app::{Action, Main};
use crate::dates;
use crate::dialogs::{area, assignees, field, label, milestones, pick, priorities, statuses};
use crate::i18n::{tr, trf};
use crate::pages::sealed_title;
use crate::widgets::{avatar, icon, icon_button, progress_bar};

pub fn show(ui: &mut Ui, main: &mut Main, look: &Look, actions: &mut Vec<Action>) {
    let inner = ui.max_rect().shrink2(egui::vec2(16.0, 12.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
        panel(ui, main, look, actions)
    });
}

fn panel(ui: &mut Ui, main: &mut Main, look: &Look, actions: &mut Vec<Action>) {
    let p = &look.palette;
    let Some(id) = main.task else {
        return;
    };
    if main.model.task(id).is_none() {
        actions.push(Action::CloseTask);
        return;
    }
    let editable = main
        .editor
        .as_ref()
        .is_some_and(|e| e.readable && main.can_edit(e.project));

    ui.horizontal(|ui| {
        ui.label(
            RichText::new(tr("Task"))
                .size(font_size::SMALL)
                .strong()
                .color(color(p.text_secondary)),
        );
        crate::widgets::trailing(ui, |ui| {
            if icon_button(ui, look, "close", tr("Close"), false).clicked() {
                actions.push(Action::CloseTask);
            }
            if editable && icon_button(ui, look, "delete", tr("Delete task"), false).clicked() {
                actions.push(Action::AskDeleteTask(id));
            }
        });
    });
    ui.add_space(4.0);

    egui::ScrollArea::vertical()
        .id_salt("task-detail")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            fields(ui, main, look, editable, actions);
            ui.add_space(16.0);
            comments(ui, main, look, actions);
        });
}

fn fields(ui: &mut Ui, main: &mut Main, look: &Look, editable: bool, actions: &mut Vec<Action>) {
    let p = &look.palette;
    let assignee_options = assignees(main);
    let Some(project) = main.editor.as_ref().map(|e| e.project) else {
        return;
    };
    let milestone_options = milestones(main, project);
    let Some(editor) = main.editor.as_mut() else {
        return;
    };
    if !editor.readable {
        ui.horizontal(|ui| {
            icon(ui, "lock", 18.0, color(p.text_secondary));
            ui.label(
                RichText::new(sealed_title())
                    .strong()
                    .color(color(p.text_primary)),
            );
        });
        ui.label(
            RichText::new(tr("Another member's device shares them as soon as it is online. Nothing here can be read or changed until then."))
                .size(font_size::SMALL)
                .color(color(p.text_secondary)),
        );
        return;
    }
    let before = editor.clone();

    ui.add_enabled_ui(editable, |ui| {
        cr::field_frame(look)
            .inner_margin(egui::Margin::symmetric(8, 6))
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut editor.title)
                        .frame(egui::Frame::NONE)
                        .font(egui::FontId::proportional(font_size::LARGE))
                        .desired_rows(1)
                        .desired_width(ui.available_width()),
                );
            });

        ui.columns(2, |cols| {
            label(&mut cols[0], look, tr("Status"));
            pick(&mut cols[0], "task-status", &mut editor.status, &statuses());
            label(&mut cols[1], look, tr("Priority"));
            pick(
                &mut cols[1],
                "task-priority",
                &mut editor.priority,
                &priorities(),
            );
        });
        label(ui, look, tr("Assignee"));
        pick(ui, "task-assignee", &mut editor.assignee, &assignee_options);
        label(ui, look, tr("Milestone"));
        pick(
            ui,
            "task-milestone",
            &mut editor.milestone,
            &milestone_options,
        );
        label(ui, look, tr("Due date"));
        field(ui, look, &mut editor.due, "YYYY-MM-DD", false);
        if dates::parse_date(&editor.due).is_none() {
            ui.label(
                RichText::new(tr("Write the date as YYYY-MM-DD."))
                    .size(font_size::SMALL)
                    .color(color(p.status_danger_fg)),
            );
        }
        label(ui, look, tr("Labels"));
        field(ui, look, &mut editor.labels, tr("design, backend"), false);
        label(ui, look, tr("Description"));
        area(
            ui,
            look,
            &mut editor.description,
            tr("Add details, links or notes"),
            4,
        );

        label(ui, look, tr("Checklist"));
        if !editor.checklist.is_empty() {
            let done = editor.checklist.iter().filter(|i| i.done).count() as u32;
            let total = editor.checklist.len() as u32;
            progress_bar(ui, look, hito_core::Progress { done, total }, 0.0, true);
        }
        let mut remove = None;
        for (i, item) in editor.checklist.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.checkbox(&mut item.done, "");
                let width = ui.available_width() - 32.0;
                ui.add(
                    egui::TextEdit::singleline(&mut item.text)
                        .frame(egui::Frame::NONE)
                        .desired_width(width.max(40.0)),
                );
                if icon_button(ui, look, "close", tr("Remove"), false).clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            editor.checklist.remove(i);
        }
        let r = field(ui, look, &mut editor.new_item, tr("Add an item"), false);
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let text = editor.new_item.trim().to_string();
            if !text.is_empty() {
                editor.checklist.push(ChecklistItem { text, done: false });
                editor.new_item.clear();
                r.request_focus();
            }
        }
    });

    let changed = editor.title != before.title
        || editor.description != before.description
        || editor.labels != before.labels
        || editor.checklist != before.checklist
        || editor.status != before.status
        || editor.priority != before.priority
        || editor.assignee != before.assignee
        || editor.milestone != before.milestone
        || editor.due != before.due;
    if changed {
        editor.dirty = true;
    }
    if editable {
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let ok = editor.dirty && !editor.saving && !editor.title.trim().is_empty();
            let text = if editor.saving {
                tr("Saving…")
            } else {
                tr("Save")
            };
            if cr::primary_button(ui, look, text, ok).clicked() {
                actions.push(Action::SaveTask);
            }
            if editor.dirty && !editor.saving {
                ui.label(
                    RichText::new(tr("Unsaved changes"))
                        .size(font_size::SMALL)
                        .color(color(p.text_secondary)),
                );
            }
        });
        if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S)) {
            actions.push(Action::SaveTask);
        }
    }
}

fn comments(ui: &mut Ui, main: &mut Main, look: &Look, actions: &mut Vec<Action>) {
    let p = &look.palette;
    let Some(editor) = &main.editor else {
        return;
    };
    let (id, project) = (editor.id, editor.project);
    let list = main.model.comments.get(&id).cloned().unwrap_or_default();
    ui.label(
        RichText::new(trf("Comments ({})", &[&list.len().to_string()]))
            .strong()
            .color(color(p.text_primary)),
    );
    ui.add_space(6.0);
    let role = main.role_in(project);
    let me = main.model.me.id;
    for c in &list {
        ui.horizontal(|ui| {
            avatar(ui, look, c.author.id.0, &c.author.display_name, 24.0, None);
            ui.label(
                RichText::new(&c.author.display_name)
                    .strong()
                    .color(color(p.text_primary)),
            );
            ui.label(
                RichText::new(dates::when(c.created_at))
                    .size(font_size::SMALL)
                    .color(color(p.text_secondary)),
            );
            if role.is_some_and(|r| can_delete_comment(me, c.author.id, r)) {
                crate::widgets::trailing(ui, |ui| {
                    if icon_button(ui, look, "delete", tr("Delete comment"), false).clicked() {
                        actions.push(Action::DeleteComment(c.id));
                    }
                });
            }
        });
        let text = main.model.comment_text.get(&c.id);
        ui.label(
            RichText::new(text.map(String::as_str).unwrap_or(sealed_title())).color(color(
                if text.is_some() {
                    p.text_primary
                } else {
                    p.text_tertiary
                },
            )),
        );
        ui.add_space(8.0);
    }
    let archived = main.model.project(project).is_some_and(|p| p.archived);
    let guest_added = main
        .model
        .project(project)
        .is_some_and(|p| p.guests.contains(&me));
    let can_comment = editor.readable
        && role.is_some_and(|r| hito_core::roles::can_comment(r, guest_added, archived));
    if !can_comment {
        return;
    }
    area(ui, look, &mut main.comment, tr("Write a comment"), 2);
    ui.horizontal(|ui| {
        let ok = !main.comment.trim().is_empty();
        if cr::secondary_button(ui, look, tr("Comment"), ok).clicked() && ok {
            actions.push(Action::PostComment);
        }
    });
}
