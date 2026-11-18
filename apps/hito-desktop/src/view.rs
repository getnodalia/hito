//! The signed-in window: the top bar, and three islands. Left: the workspace, Overview, My tasks
//! and the projects with their progress. Centre: the page (`pages.rs`). Right, when a task is
//! open: its details (`detail.rs`).

use std::sync::Arc;

use egui::{
    pos2, vec2, Align, Color32, CornerRadius, Id, Key, Layout, Margin, Modifiers, Rect, RichText,
    Sense, Stroke, Ui, UiBuilder,
};
use nodalia_theme::creator::{self as cr, layout, Look};
use nodalia_theme::for_egui::color;
use nodalia_theme::{font_size, Mode};

use crate::app::{Action, App, Backstage, Main, Screen};
use crate::chrome;
use crate::dialogs::{self, Dialog};
use crate::i18n::{tr, trf};
use crate::model::{Connection, Page, Tab};
use crate::widgets::{avatar, badge, icon, icon_button, progress_bar};
use crate::{detail, pages};

const SIDEBAR: f32 = 260.0;
const DETAIL: f32 = 360.0;

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

        let bar = full.with_max_y(full.top() + layout::APP_BAR);
        top_bar(ui, bar, main, &look, mode, ndl, &mut actions);

        let gap = layout::GAP;
        let body = Rect::from_min_max(
            pos2(full.left() + gap, bar.bottom()),
            pos2(full.right() - gap, full.bottom() - gap),
        );
        let side = Rect::from_min_max(body.min, pos2(body.left() + SIDEBAR, body.bottom()));
        let wide = body.width() > SIDEBAR + DETAIL + 360.0;
        let show_detail = main.task.is_some() && wide;
        let right = Rect::from_min_max(pos2(body.right() - DETAIL, body.top()), body.max);
        let center_right = if show_detail {
            right.left() - gap
        } else {
            body.right()
        };
        let center = Rect::from_min_max(
            pos2(side.right() + gap, body.top()),
            pos2(center_right, body.bottom()),
        );

        island(ui, &look, side, "sidebar", |ui| {
            sidebar(ui, main, &look, &mut actions)
        });
        if main.task.is_some() && !wide {
            // Too narrow for three islands: the task covers the page.
            island(ui, &look, center, "detail-wide", |ui| {
                detail::show(ui, main, &look, &mut actions)
            });
        } else {
            island(ui, &look, center, "center", |ui| {
                pages::show(ui, main, &look, now, &mut actions)
            });
        }
        if show_detail {
            island(ui, &look, right, "detail", |ui| {
                detail::show(ui, main, &look, &mut actions)
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

pub fn island(ui: &mut Ui, look: &Look, rect: Rect, id: &str, add: impl FnOnce(&mut Ui)) {
    ui.scope_builder(UiBuilder::new().max_rect(rect).id_salt(id), |ui| {
        cr::island(look).show(ui, |ui| {
            ui.set_min_size(rect.size());
            ui.set_max_size(rect.size());
            add(ui);
        });
    });
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
        actions.push(Action::Backstage(Some(Backstage::Preferences)));
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
    let p = &look.palette;
    if chrome::drags_window(ndl) {
        chrome::drag_area(ui, bar);
    }
    let right_reserved = if ndl {
        chrome::CONTROLS_WIDTH
    } else {
        layout::GAP
    };
    let inner = Rect::from_min_max(
        pos2(
            bar.left() + layout::GAP + chrome::leading_inset(),
            bar.top(),
        ),
        pos2(bar.right() - right_reserved, bar.bottom()),
    );
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            let menu = icon_button(ui, look, "menu", tr("Main menu"), false);
            egui::Popup::menu(&menu).show(|ui| {
                ui.set_min_width(220.0);
                if ui.button(tr("Home")).clicked() {
                    actions.push(Action::Backstage(Some(Backstage::Home)));
                }
                if ui.button(tr("API tokens and webhooks")).clicked() {
                    actions.push(Action::OpenIntegrations);
                }
                ui.separator();
                if ui.button(tr("Preferences")).clicked() {
                    actions.push(Action::Backstage(Some(Backstage::Preferences)));
                }
                if ui.button(tr("Keyboard shortcuts")).clicked() {
                    actions.push(Action::Backstage(Some(Backstage::Shortcuts)));
                }
                if ui.button(tr("About Hito")).clicked() {
                    actions.push(Action::Backstage(Some(Backstage::About)));
                }
                ui.separator();
                if ui.button(tr("Log out")).clicked() {
                    actions.push(Action::Logout);
                }
            });
            ui.add_space(4.0);
            logo(ui, look, 22.0);
            ui.label(
                RichText::new("Hito")
                    .strong()
                    .size(font_size::LARGE)
                    .color(color(p.text_primary)),
            );

            // Search, centred in what is left.
            let rest = ui.available_width();
            let width = (rest - 300.0).clamp(160.0, 420.0);
            ui.add_space(((rest - width) / 2.0 - 150.0).max(8.0));
            let response = crate::widgets::search_field(
                ui,
                look,
                &mut main.search,
                tr("Filter tasks by title or label"),
                width,
            );
            if main.focus_search {
                response.request_focus();
                main.focus_search = false;
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
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
        },
    );
    if ndl {
        chrome::controls(ui, bar, p, mode);
    }
}

fn connection_pill(ui: &mut Ui, look: &Look, connection: &Connection) {
    let p = &look.palette;
    let (text, fg, bg) = match connection {
        Connection::Connected => (
            tr("Connected").to_string(),
            p.status_success_fg,
            p.status_success_bg,
        ),
        Connection::Connecting => (
            tr("Connecting…").to_string(),
            p.status_info_fg,
            p.status_info_bg,
        ),
        Connection::Retrying(s) => (
            trf("Disconnected. Retrying in {} s.", &[&s.to_string()]),
            p.status_warning_fg,
            p.status_warning_bg,
        ),
    };
    egui::Frame::NONE
        .fill(color(bg))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(10, 3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (dot, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                ui.painter().circle_filled(dot.center(), 4.0, color(fg));
                ui.label(RichText::new(text).size(font_size::SMALL).color(color(fg)));
            });
        });
}

/// The app icon, drawn from the embedded PNG.
pub fn logo(ui: &mut Ui, _look: &Look, size: f32) {
    let texture = crate::icon::texture(ui.ctx());
    ui.add(
        egui::Image::new(&texture)
            .fit_to_exact_size(vec2(size, size))
            .corner_radius(CornerRadius::same((size * 0.22) as u8)),
    );
}

// --- Sidebar -----------------------------------------------------------------------------------

fn sidebar(ui: &mut Ui, main: &mut Main, look: &Look, actions: &mut Vec<Action>) {
    let p = &look.palette;
    let full = ui.max_rect();
    let footer_h = 52.0;
    let top = Rect::from_min_max(full.min, pos2(full.right(), full.bottom() - footer_h));
    let footer = Rect::from_min_max(pos2(full.left(), full.bottom() - footer_h), full.max);

    ui.scope_builder(
        UiBuilder::new().max_rect(top.shrink2(vec2(8.0, 8.0))),
        |ui| {
            workspace_switcher(ui, main, look, actions);
            ui.add_space(6.0);
            if main.model.current_workspace().is_none() {
                return;
            }
            if cr::home_row(
                ui,
                look,
                "home",
                tr("Overview"),
                main.page == Page::Overview,
            )
            .clicked()
            {
                actions.push(Action::ShowPage(Page::Overview));
            }
            ui.add_space(2.0);
            let open = main.model.open_my_tasks().len();
            let row = cr::home_row(
                ui,
                look,
                "check-square",
                tr("My tasks"),
                main.page == Page::MyTasks,
            );
            if open > 0 {
                let mut child = ui.new_child(
                    UiBuilder::new()
                        .max_rect(row.rect.shrink2(vec2(8.0, 0.0)))
                        .layout(Layout::right_to_left(Align::Center)),
                );
                badge(
                    &mut child,
                    look,
                    u32::try_from(open).unwrap_or(u32::MAX),
                    false,
                );
            }
            if row.clicked() {
                actions.push(Action::ShowPage(Page::MyTasks));
            }
            ui.add_space(12.0);

            let can_create = main
                .model
                .current_workspace()
                .is_some_and(|w| w.role.can_create_project());
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                ui.label(
                    RichText::new(tr("Projects"))
                        .size(font_size::SMALL)
                        .strong()
                        .color(color(p.text_secondary)),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if can_create
                        && icon_button(ui, look, "plus", tr("New project"), false).clicked()
                    {
                        actions.push(Action::Dialog(Dialog::new_project()));
                    }
                });
            });
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
        },
    );

    // Who is signed in.
    ui.painter().line_segment(
        [
            pos2(footer.left() + 12.0, footer.top()),
            pos2(footer.right() - 12.0, footer.top()),
        ],
        Stroke::new(1.0, color(p.border_subtle)),
    );
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(footer.shrink2(vec2(12.0, 8.0)))
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            let me = &main.model.me;
            avatar(ui, look, me.id.0, &me.display_name, 30.0, Some(true));
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(&me.display_name)
                            .strong()
                            .color(color(p.text_primary)),
                    )
                    .truncate(),
                );
                ui.label(
                    RichText::new(format!("@{}", me.username))
                        .size(font_size::SMALL)
                        .color(color(p.text_secondary)),
                );
            });
        },
    );
}

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
    let fill = if selected {
        color(p.accent_subtle)
    } else if response.hovered() {
        color(p.control_bg_hover)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter()
        .rect_filled(rect, CornerRadius::same(layout::CONTROL_RADIUS), fill);
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(layout::CONTROL_RADIUS),
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
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect.shrink2(vec2(10.0, 6.0))));
    child.horizontal(|ui| {
        icon(ui, if archived { "archive" } else { "folder" }, 15.0, ink);
        ui.add_space(4.0);
        let mut text = RichText::new(name).color(ink);
        if selected {
            text = text.strong();
        }
        ui.add(egui::Label::new(text).truncate().selectable(false));
    });
    child.add_space(2.0);
    progress_bar(&mut child, look, progress, 0.0, true);
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, name));
    response
}

fn workspace_switcher(ui: &mut Ui, main: &Main, look: &Look, actions: &mut Vec<Action>) {
    let p = &look.palette;
    let model = &main.model;
    let name = model
        .current_workspace()
        .map_or(tr("Workspaces").to_string(), |w| w.name.clone());
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
    let fill = if response.hovered() {
        color(p.control_bg_hover)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter()
        .rect_filled(rect, CornerRadius::same(layout::CONTROL_RADIUS), fill);
    let tile = Rect::from_min_size(rect.min + vec2(6.0, 6.0), vec2(28.0, 28.0));
    ui.painter()
        .rect_filled(tile, CornerRadius::same(8), color(p.accent_default));
    let initial: String = name
        .chars()
        .next()
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_default();
    ui.painter().text(
        tile.center(),
        egui::Align2::CENTER_CENTER,
        initial,
        egui::FontId::proportional(15.0),
        color(p.text_on_accent),
    );
    ui.painter().text(
        pos2(tile.right() + 10.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        &name,
        egui::FontId::proportional(font_size::LARGE),
        color(p.text_primary),
    );
    if let Some(chevron) = nodalia_theme::icons::get("chevron-down") {
        let r =
            Rect::from_center_size(pos2(rect.right() - 16.0, rect.center().y), vec2(14.0, 14.0));
        nodalia_theme::icons::paint(
            ui.painter(),
            r,
            chevron,
            nodalia_theme::icons::Style::bold(color(p.text_secondary)),
        );
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tr("Workspaces")));
    egui::Popup::menu(&response).show(|ui| {
        ui.set_min_width(rect.width() - 8.0);
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
    ui.add_space(40.0);
    ui.vertical_centered(|ui| {
        icon(ui, glyph, 40.0, color(look.palette.text_tertiary));
        ui.add_space(10.0);
        ui.label(RichText::new(text).color(color(look.palette.text_secondary)));
    });
}

pub fn toasts(ui: &mut Ui, app: &App, look: &Look) {
    let p = &look.palette;
    let full = ui.max_rect();
    let mut y = full.bottom() - 24.0;
    for toast in app.toasts.iter().rev() {
        let galley = ui.painter().layout(
            toast.text.clone(),
            egui::FontId::proportional(font_size::BODY),
            color(p.text_inverse),
            340.0,
        );
        let size = galley.size() + vec2(28.0, 18.0);
        let rect = Rect::from_min_size(pos2(full.center().x - size.x / 2.0, y - size.y), size);
        let painter = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            Id::unique("toasts"),
        ));
        painter.rect_filled(rect, CornerRadius::same(12), color(p.surface_inverse));
        painter.galley(
            rect.min + vec2(14.0, 9.0),
            Arc::clone(&galley),
            color(p.text_inverse),
        );
        y = rect.top() - 8.0;
    }
}
