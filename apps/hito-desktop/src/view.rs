//! The signed-in window: the top bar, and three islands. Left: the workspace, Overview, My tasks
//! and the projects with their progress. Centre: the page (`pages.rs`). Right, when a task is
//! open: its details (`detail.rs`).

use egui::{pos2, vec2, Color32, CornerRadius, Key, Modifiers, Rect, RichText, Sense, Stroke, Ui};
use nodalia_theme::creator::{self as cr, Look};
use nodalia_theme::enterprise::{self as ent, Account, Count, Panes, Row, Tone};
use nodalia_theme::for_egui::color;
use nodalia_theme::{font_size, window_controls, Mode};

use crate::app::{Action, App, Backstage, Main, Screen};
use crate::chrome;
use crate::dialogs::{self, Dialog};
use crate::i18n::{tr, trf};
use crate::model::{Connection, Page, Tab};
use crate::{detail, pages};

pub fn show(ui: &mut Ui, app: &mut App, now: f64) -> Vec<Action> {
    let mut actions = Vec::new();
    let ctx = ui.ctx().clone();
    let (look, mode) = app.look(&ctx);
    let ndl = app.ndl_controls();
    let full = ui.max_rect();
    ui.painter()
        .rect_filled(full, 0.0, color(look.palette.surface_window));

    {
        let App {
            screen, settings, ..
        } = app;
        let Screen::Main(main) = screen else {
            return actions;
        };
        shortcuts(&ctx, main, &mut actions);

        let show_detail = main.task.is_some();
        let panes = Panes::new(full, show_detail.then_some(ent::layout::DETAIL));
        top_bar(ui, panes.bar, main, &look, mode, ndl, &mut actions);
        let (side, center) = (panes.side, panes.center);

        ent::island(ui, &look, side, "sidebar", |ui| {
            sidebar(ui, main, &look, &mut actions)
        });
        if panes.squeezed {
            // Too narrow for three islands: the task covers the page.
            ent::island(ui, &look, center, "detail-wide", |ui| {
                detail::show(ui, main, &look, &mut actions)
            });
        } else if let Some(right) = panes.detail {
            ent::island(ui, &look, center, "center", |ui| {
                pages::show(ui, main, &look, now, &mut actions)
            });
            ent::island(ui, &look, right, "detail", |ui| {
                detail::show(ui, main, &look, &mut actions)
            });
        } else {
            ent::island(ui, &look, center, "center", |ui| {
                pages::show(ui, main, &look, now, &mut actions)
            });
        }
        dialogs::show(&ctx, main, settings, &look, &mut actions);
    }
    toasts(ui, app, &look);
    if ndl {
        chrome::resize_edges(ui, full);
    }
    actions
}

fn shortcuts(ctx: &egui::Context, main: &mut Main, actions: &mut Vec<Action>) {
    if main.dialog.is_some() {
        return;
    }
    let (search, prefs, new_task, refresh, escape, tabs) = ctx.input_mut(|i| {
        (
            i.consume_key(Modifiers::COMMAND, Key::K),
            i.consume_key(Modifiers::COMMAND, Key::Comma),
            i.consume_key(Modifiers::COMMAND, Key::N),
            i.consume_key(Modifiers::NONE, Key::F5),
            i.key_pressed(Key::Escape),
            [Key::Num1, Key::Num2, Key::Num3, Key::Num4]
                .map(|k| i.consume_key(Modifiers::COMMAND, k)),
        )
    });
    if search {
        main.focus_search = true;
    }
    if prefs {
        actions.push(Action::Dialog(Dialog::Preferences));
    }
    if new_task {
        actions.push(Action::NewTaskDialog);
    }
    if refresh {
        actions.push(Action::Refresh);
    }
    if escape && main.task.is_some() {
        actions.push(Action::CloseTask);
    }
    let order = [Tab::Board, Tab::List, Tab::Milestones, Tab::Activity];
    for (pressed, tab) in tabs.into_iter().zip(order) {
        if pressed {
            actions.push(Action::ShowTab(tab));
        }
    }
}

// --- Top bar -----------------------------------------------------------------------------------

fn top_bar(
    ui: &mut Ui,
    bar: Rect,
    main: &mut Main,
    look: &Look,
    mode: Mode,
    ndl: bool,
    actions: &mut Vec<Action>,
) {
    let labels = window_controls::Labels {
        minimize: tr("Minimize"),
        maximize: tr("Maximize"),
        restore: tr("Restore"),
        close: tr("Close"),
    };
    ent::top_bar(ui, look, mode, ndl, bar, labels, |ui| {
        ent::main_menu(ui, look, tr("Main menu"), |ui| {
            if ui.button(tr("Home")).clicked() {
                actions.push(Action::Backstage(Some(Backstage::Home)));
            }
            if ui.button(tr("API tokens and webhooks")).clicked() {
                actions.push(Action::OpenIntegrations);
            }
            ui.separator();
            if ui.button(tr("Preferences")).clicked() {
                actions.push(Action::Dialog(Dialog::Preferences));
            }
            if ui.button(tr("Keyboard shortcuts")).clicked() {
                actions.push(Action::Dialog(Dialog::Shortcuts));
            }
            if ui.button(tr("About Hito")).clicked() {
                actions.push(Action::Dialog(Dialog::About));
            }
            if ui.button(tr("Check for updates…")).clicked() {
                actions.push(Action::CheckForUpdates);
            }
            ui.separator();
            if ui.button(tr("Log out")).clicked() {
                actions.push(Action::Logout);
            }
        });
        ent::brand(ui, look, &crate::icon::texture(ui.ctx()), "Hito");

        let response = ent::bar_search(
            ui,
            look,
            &mut main.search,
            tr("Filter tasks by title or label"),
            300.0,
        );
        if main.focus_search {
            response.request_focus();
            main.focus_search = false;
        }

        ent::trailing(ui, |ui| {
            let can_add = main
                .model
                .current_projects()
                .iter()
                .any(|p| main.can_edit(p.id));
            if cr::primary_button(ui, look, tr("New task"), can_add).clicked() {
                actions.push(Action::NewTaskDialog);
            }
            ui.add_space(8.0);
            connection_pill(ui, look, &main.model.connection);
        });
    });
}

fn connection_pill(ui: &mut Ui, look: &Look, connection: &Connection) {
    let (tone, text) = match connection {
        Connection::Connected => (Tone::Success, tr("Connected").to_string()),
        Connection::Connecting => (Tone::Info, tr("Connecting…").to_string()),
        Connection::Retrying(s) => (
            Tone::Warning,
            trf("Disconnected. Retrying in {} s.", &[&s.to_string()]),
        ),
    };
    ent::status_pill(ui, look, tone, &text);
}

/// The app icon, drawn from the embedded PNG.
pub fn logo(ui: &mut Ui, _look: &Look, size: f32) {
    ent::logo(ui, &crate::icon::texture(ui.ctx()), size);
}

// --- Sidebar -----------------------------------------------------------------------------------

fn sidebar(ui: &mut Ui, main: &mut Main, look: &Look, actions: &mut Vec<Action>) {
    let p = &look.palette;
    let me = &main.model.me;
    let detail = format!("@{}", me.username);
    let account = Account {
        seed: me.id.0,
        name: &me.display_name,
        detail: &detail,
        online: Some(true),
    };
    ent::sidebar(ui, look, &account, |ui| {
        workspace_switcher(ui, main, look, actions);
        ui.add_space(6.0);
        if main.model.current_workspace().is_none() {
            return;
        }
        if Row::new(tr("Overview"))
            .icon("home")
            .selected(main.page == Page::Overview)
            .show(ui, look)
            .clicked()
        {
            actions.push(Action::ShowPage(Page::Overview));
        }
        ui.add_space(2.0);
        let open = u32::try_from(main.model.open_my_tasks().len()).unwrap_or(u32::MAX);
        if Row::new(tr("My tasks"))
            .icon("check-square")
            .selected(main.page == Page::MyTasks)
            .count(Count::Quiet(open))
            .show(ui, look)
            .clicked()
        {
            actions.push(Action::ShowPage(Page::MyTasks));
        }
        ui.add_space(12.0);

        let can_create = main
            .model
            .current_workspace()
            .is_some_and(|w| w.role.can_create_project());
        let add = can_create.then(|| tr("New project"));
        if ent::section_heading(ui, look, tr("Projects"), add) {
            actions.push(Action::Dialog(Dialog::new_project()));
        }
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let projects = main.model.current_projects();
                if projects.is_empty() {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(if can_create {
                            tr("No projects yet. Create the first one with +.")
                        } else {
                            tr("No projects you can see yet.")
                        })
                        .size(font_size::SMALL)
                        .color(color(p.text_tertiary)),
                    );
                }
                let mut archived_heading = false;
                for project in projects {
                    if project.archived && !archived_heading {
                        archived_heading = true;
                        ui.add_space(10.0);
                        ui.label(
                            RichText::new(tr("Archived"))
                                .size(font_size::CAPTION)
                                .color(color(p.text_tertiary)),
                        );
                    }
                    let selected = main.page == Page::Project(project.id);
                    let progress = main.model.progress(project);
                    let row = project_row(
                        ui,
                        look,
                        &project.name,
                        selected,
                        project.archived,
                        progress,
                    );
                    if row.clicked() {
                        actions.push(Action::ShowPage(Page::Project(project.id)));
                    }
                }
            });
    });
}

/// A project in the sidebar: the suite's row (icon, name) with its progress under it. It is
/// taller than the other rows because the bar needs the room, and is painted rather than laid
/// out so it keeps its height whatever the spacing.
fn project_row(
    ui: &mut Ui,
    look: &Look,
    name: &str,
    selected: bool,
    archived: bool,
    progress: hito_core::Progress,
) -> egui::Response {
    let p = &look.palette;
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::click());
    let radius = CornerRadius::same(nodalia_theme::creator::layout::CONTROL_RADIUS);
    let fill = if selected {
        color(p.accent_subtle)
    } else if response.hovered() {
        color(p.control_bg_hover)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, radius, fill);
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            radius,
            Stroke::new(2.0, color(p.border_focus)),
            egui::StrokeKind::Inside,
        );
    }
    let ink = if selected {
        color(p.accent_text)
    } else if archived {
        color(p.text_tertiary)
    } else {
        color(p.text_primary)
    };
    let inner = rect.shrink2(vec2(10.0, 7.0));
    let painter = ui.painter();
    let glyph = Rect::from_min_size(inner.min + vec2(0.0, 1.0), vec2(15.0, 15.0));
    if let Some(i) = nodalia_theme::icons::get(if archived { "archive" } else { "folder" }) {
        nodalia_theme::icons::paint(painter, glyph, i, nodalia_theme::icons::Style::bold(ink));
    }
    let font = egui::FontId::proportional(font_size::BODY);
    let mut job = egui::text::LayoutJob::simple_singleline(name.to_string(), font, ink);
    job.wrap = egui::text::TextWrapping::truncate_at_width(inner.width() - 22.0);
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    painter.galley(pos2(glyph.right() + 7.0, inner.top()), galley, ink);
    let percent = format!("{} %", progress.percent());
    let label = painter.layout_no_wrap(
        percent,
        egui::FontId::proportional(font_size::SMALL),
        color(p.text_secondary),
    );
    let bar_y = inner.bottom() - 4.0;
    let bar = Rect::from_min_max(
        pos2(inner.left(), bar_y - 3.0),
        pos2(inner.right() - label.size().x - 8.0, bar_y + 3.0),
    );
    painter.rect_filled(bar, CornerRadius::same(3), color(p.control_bg_hover));
    let fraction = progress.fraction().clamp(0.0, 1.0);
    if fraction > 0.0 {
        let mut done = bar;
        done.set_width((bar.width() * fraction).max(6.0));
        let fill = if progress.complete() {
            color(p.status_success_fg)
        } else {
            color(p.accent_default)
        };
        painter.rect_filled(done, CornerRadius::same(3), fill);
    }
    painter.galley(
        pos2(inner.right() - label.size().x, bar_y - label.size().y / 2.0),
        label,
        color(p.text_secondary),
    );
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, name));
    response
}

fn workspace_switcher(ui: &mut Ui, main: &Main, look: &Look, actions: &mut Vec<Action>) {
    let p = &look.palette;
    let model = &main.model;
    let name = model
        .current_workspace()
        .map_or(tr("Workspaces").to_string(), |w| w.name.clone());
    ent::workspace_switcher(ui, look, &name, tr("Workspaces"), |ui| {
        ui.label(
            RichText::new(tr("Workspaces"))
                .size(font_size::CAPTION)
                .color(color(p.text_secondary)),
        );
        for w in &model.workspaces {
            let selected = model.workspace == Some(w.id);
            if ui.selectable_label(selected, &w.name).clicked() {
                actions.push(Action::SelectWorkspace(w.id));
            }
        }
        ui.separator();
        if model
            .current_workspace()
            .is_some_and(|w| w.role.can_invite())
            && ui.button(tr("Invite people")).clicked()
        {
            actions.push(Action::Dialog(Dialog::invite()));
        }
        if ui.button(tr("Create a workspace")).clicked() {
            actions.push(Action::Dialog(Dialog::CreateWorkspace {
                name: String::new(),
            }));
        }
        if ui.button(tr("Join a workspace")).clicked() {
            actions.push(Action::Dialog(Dialog::JoinWorkspace {
                code: String::new(),
            }));
        }
    });
}

// --- Shared bits -------------------------------------------------------------------------------

/// A centred hint in an empty area.
pub fn empty_state(ui: &mut Ui, look: &Look, glyph: &str, text: &str) {
    ent::empty_state(ui, look, glyph, text);
}

pub fn toasts(ui: &mut Ui, app: &App, look: &Look) {
    ent::toasts(ui, look, app.toasts.iter().map(|t| t.text.as_str()));
}
