//! The File view, drawn over the whole window: Home (start a project from a template, recent
//! projects), Preferences, Keyboard shortcuts and About. It is the Office suite's shared view
//! (`nodalia_theme::office`) under Hito's own top bar; Esc or Back returns to the work.

use egui::{pos2, vec2, Align, Key, Layout, Rect, RichText, Stroke, Ui, UiBuilder};
use nodalia_theme::font_size;
use nodalia_theme::for_egui::color;
use nodalia_theme::office;

use crate::app::{Action, App, Backstage, Screen};
use crate::chrome;
use crate::dialogs::Dialog;
use crate::i18n::{tr, trf, Lang};
use crate::model::Page;
use crate::settings::{Theme, WindowControls};
use crate::templates::Kind;
use crate::{dates, view};

const HOME: &str = "home";
const PREFERENCES: &str = "preferences";
const SHORTCUTS: &str = "shortcuts";
const ABOUT: &str = "about";

fn id(page: Backstage) -> &'static str {
    match page {
        Backstage::Home => HOME,
        Backstage::Preferences => PREFERENCES,
        Backstage::Shortcuts => SHORTCUTS,
        Backstage::About => ABOUT,
    }
}

fn label(page: Backstage) -> &'static str {
    match page {
        Backstage::Home => tr("Home"),
        Backstage::Preferences => tr("Preferences"),
        Backstage::Shortcuts => tr("Keyboard shortcuts"),
        Backstage::About => tr("About Hito"),
    }
}

pub fn show(ui: &mut Ui, app: &mut App) -> Vec<Action> {
    let mut actions = Vec::new();
    let ctx = ui.ctx().clone();
    let (look, mode) = app.look(&ctx);
    let ndl = app.ndl_controls();
    let full = ui.max_rect();
    let p = look.palette;
    ui.painter().rect_filled(full, 0.0, color(p.surface_window));

    let Screen::Main(main) = &app.screen else {
        return actions;
    };
    let Some(page) = main.backstage else {
        return actions;
    };

    // The app bar: the logo and name, and the window controls.
    let bar = full.with_max_y(full.top() + nodalia_theme::creator::layout::APP_BAR);
    if chrome::drags_window(ndl) {
        chrome::drag_area(ui, bar);
    }
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(bar.with_min_x(bar.left() + 12.0 + chrome::leading_inset()))
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            view::logo(ui, &look, 22.0);
            ui.label(
                RichText::new("Hito")
                    .strong()
                    .size(font_size::LARGE)
                    .color(color(p.text_primary)),
            );
        },
    );
    if ndl {
        chrome::controls(ui, bar, &p, mode);
    }

    let typing = ctx.memory(|m| m.focused().is_some());
    if !typing && ctx.input(|i| i.key_pressed(Key::Escape)) {
        actions.push(Action::Backstage(None));
    }

    let office_look = office::Look::new(p);
    let item =
        |page: Backstage, icon: &'static str| office::Entry::item(id(page), label(page), icon);
    let nav = office::Nav {
        back: tr("Back"),
        top: vec![item(Backstage::Home, "home")],
        bottom: vec![
            item(Backstage::Shortcuts, "keyboard"),
            item(Backstage::About, "info"),
            item(Backstage::Preferences, "settings"),
        ],
        selected: Some(id(page)),
    };
    let body = Rect::from_min_max(pos2(full.left(), bar.bottom()), full.max);
    let mut picked = None;
    ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
        picked = office::file_view(ui, &office_look, &nav, |ui| {
            office::page(ui, &office_look, label(page), |ui| match page {
                Backstage::Home => home(ui, app, &office_look, &mut actions),
                Backstage::Preferences => preferences(ui, app, &office_look, &mut actions),
                Backstage::Shortcuts => shortcuts(ui, &office_look),
                Backstage::About => about(ui, app, &office_look),
            });
        });
    });
    match picked.as_deref() {
        Some(office::BACK) => actions.push(Action::Backstage(None)),
        Some(HOME) => actions.push(Action::Backstage(Some(Backstage::Home))),
        Some(PREFERENCES) => actions.push(Action::Backstage(Some(Backstage::Preferences))),
        Some(SHORTCUTS) => actions.push(Action::Backstage(Some(Backstage::Shortcuts))),
        Some(ABOUT) => actions.push(Action::Backstage(Some(Backstage::About))),
        _ => {}
    }
    actions
}

/// A little timeline for each template: one bar per milestone, staggered like a Gantt chart.
fn draw_template(kind: Kind) -> impl Fn(&egui::Painter, Rect) {
    move |painter: &egui::Painter, rect: Rect| {
        let accent = egui::Color32::from_rgb(0x3D, 0x9A, 0x2B);
        let soft = accent.gamma_multiply(0.35);
        let bars: &[(f32, f32)] = match kind {
            Kind::Blank => &[],
            Kind::Launch => &[(0.0, 0.4), (0.3, 0.75), (0.7, 1.0)],
            Kind::Client => &[(0.0, 0.25), (0.2, 0.6), (0.55, 0.85), (0.8, 1.0)],
            Kind::Sprint => &[(0.0, 0.8), (0.75, 1.0)],
            Kind::Event => &[(0.0, 0.5), (0.35, 0.8), (0.85, 1.0)],
            Kind::Hiring => &[(0.0, 0.3), (0.25, 0.8), (0.75, 1.0)],
            Kind::Onboarding => &[(0.0, 0.2), (0.15, 0.5), (0.45, 1.0)],
            Kind::Campaign => &[(0.0, 0.3), (0.25, 0.7), (0.65, 1.0)],
            Kind::Website => &[(0.0, 0.35), (0.3, 0.75), (0.7, 1.0)],
            Kind::Quarter => &[(0.0, 0.34), (0.33, 0.67), (0.66, 1.0)],
        };
        let rows = 4.0;
        let h = rect.height() / rows;
        for i in 0..4 {
            let y = rect.top() + h * (i as f32 + 0.5);
            painter.line_segment(
                [pos2(rect.left(), y), pos2(rect.right(), y)],
                Stroke::new(1.0, soft.gamma_multiply(0.5)),
            );
        }
        for (i, (a, b)) in bars.iter().enumerate() {
            let y = rect.top() + h * (i as f32 + 0.5);
            let bar = Rect::from_min_max(
                pos2(rect.left() + rect.width() * a, y - h * 0.28),
                pos2(rect.left() + rect.width() * b, y + h * 0.28),
            );
            painter.rect_filled(bar, 3.0, if i == 0 { accent } else { soft });
        }
        if bars.is_empty() {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "+",
                egui::FontId::proportional(32.0),
                accent,
            );
        }
    }
}

fn home(ui: &mut Ui, app: &App, look: &office::Look, actions: &mut Vec<Action>) {
    let Screen::Main(main) = &app.screen else {
        return;
    };
    let p = &look.palette;
    ui.label(
        RichText::new(trf("Hello, {}.", &[&main.model.me.display_name]))
            .size(font_size::LARGE)
            .color(color(p.text_primary)),
    );
    let can_create = main
        .model
        .current_workspace()
        .is_some_and(|w| w.role.can_create_project());
    office::section(ui, look, tr("Start a project"), |ui| {
        if !can_create {
            office::hint(
                ui,
                look,
                tr("Guests cannot create projects. Ask an admin of the workspace."),
            );
            return;
        }
        let draws: Vec<_> = Kind::ALL.iter().map(|k| draw_template(*k)).collect();
        let list: Vec<office::Template> = Kind::ALL
            .iter()
            .zip(&draws)
            .map(|(k, d)| office::Template {
                name: k.name(),
                draw: d,
            })
            .collect();
        if let Some(i) = office::templates(ui, look, &list) {
            if let Some(kind) = Kind::ALL.get(i) {
                actions.push(Action::Backstage(None));
                actions.push(Action::Dialog(Dialog::NewProject {
                    name: String::new(),
                    description: String::new(),
                    template: *kind,
                }));
            }
        }
    });
    office::section(ui, look, tr("Recent projects"), |ui| {
        let mut projects: Vec<_> = main
            .model
            .current_projects()
            .iter()
            .filter(|p| !p.archived)
            .collect();
        projects.sort_by_key(|p| std::cmp::Reverse(p.updated_at));
        let workspace = main
            .model
            .current_workspace()
            .map(|w| w.name.clone())
            .unwrap_or_default();
        let whens: Vec<String> = projects
            .iter()
            .map(|p| {
                trf(
                    "{} % done · {}",
                    &[
                        &p.progress.percent().to_string(),
                        &dates::when(p.updated_at),
                    ],
                )
            })
            .collect();
        let list: Vec<office::Recent> = projects
            .iter()
            .zip(&whens)
            .take(8)
            .map(|(p, w)| office::Recent {
                name: &p.name,
                folder: &workspace,
                when: w,
                path: &p.name,
            })
            .collect();
        if let Some(i) = office::recents(
            ui,
            look,
            &list,
            tr("No projects yet. Start one from a template above."),
            Some(tr("Progress")),
        ) {
            if let Some(project) = projects.get(i) {
                actions.push(Action::Backstage(None));
                actions.push(Action::ShowPage(Page::Project(project.id)));
            }
        }
    });
}

fn preferences(ui: &mut Ui, app: &mut App, look: &office::Look, actions: &mut Vec<Action>) {
    let settings = &mut app.settings;
    let before = settings.clone();
    office::section(ui, look, tr("Appearance"), |ui| {
        office::settings(ui, "prefs-appearance", |ui| {
            office::setting(ui, look, tr("Theme"), |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut settings.theme, Theme::System, tr("System"));
                    ui.selectable_value(&mut settings.theme, Theme::Light, tr("Light"));
                    ui.selectable_value(&mut settings.theme, Theme::Dark, tr("Dark"));
                });
            });
            office::setting(ui, look, tr("High contrast"), |ui| {
                ui.checkbox(&mut settings.high_contrast, "");
            });
            office::setting(ui, look, tr("Reduce motion"), |ui| {
                ui.checkbox(&mut settings.reduced_motion, "");
            });
        });
    });
    if chrome::NDL_CONTROLS_POSSIBLE {
        office::section(ui, look, tr("Window"), |ui| {
            office::settings(ui, "prefs-window", |ui| {
                office::setting(ui, look, tr("Window buttons"), |ui| {
                    ui.horizontal(|ui| {
                        ui.selectable_value(
                            &mut settings.window_controls,
                            WindowControls::Nodalia,
                            tr("Nodalia"),
                        );
                        ui.selectable_value(
                            &mut settings.window_controls,
                            WindowControls::System,
                            tr("The desktop's own"),
                        );
                    });
                });
            });
        });
    }
    office::section(ui, look, tr("Language"), |ui| {
        office::settings(ui, "prefs-language", |ui| {
            office::setting(ui, look, tr("Interface language"), |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut settings.language, None, tr("System"));
                    ui.selectable_value(&mut settings.language, Some(Lang::Es), Lang::Es.name());
                    ui.selectable_value(&mut settings.language, Some(Lang::En), Lang::En.name());
                });
            });
        });
        office::hint(
            ui,
            look,
            tr("Project templates are written in the language chosen when the project is created."),
        );
    });
    office::section(ui, look, tr("Account"), |ui| {
        office::settings(ui, "prefs-account", |ui| {
            office::setting(ui, look, tr("Stay signed in on this computer"), |ui| {
                ui.checkbox(&mut settings.remember, "");
            });
        });
    });
    #[cfg(target_os = "linux")]
    office::section(ui, look, tr("Notifications"), |ui| {
        office::settings(ui, "prefs-notifications", |ui| {
            office::setting(ui, look, tr("When a task is assigned to me"), |ui| {
                ui.checkbox(&mut settings.notifications, "");
            });
        });
        office::hint(
            ui,
            look,
            tr("Notifications never include a task's title or its project."),
        );
    });
    if *settings != before {
        actions.push(Action::SettingsChanged);
    }
}

fn shortcuts(ui: &mut Ui, look: &office::Look) {
    let p = &look.palette;
    let command = if cfg!(target_os = "macos") {
        "⌘"
    } else {
        "Ctrl"
    };
    let rows = [
        (tr("New task"), format!("{command}+N")),
        (tr("Filter tasks"), format!("{command}+K")),
        (tr("Board"), format!("{command}+1")),
        (tr("List"), format!("{command}+2")),
        (tr("Milestones"), format!("{command}+3")),
        (tr("Activity"), format!("{command}+4")),
        (tr("Save the open task"), format!("{command}+S")),
        (tr("Close the task or this view"), "Esc".to_string()),
        (tr("Refresh"), "F5".to_string()),
        (tr("Preferences"), format!("{command}+,")),
    ];
    egui::Grid::new("shortcuts")
        .num_columns(2)
        .spacing(vec2(32.0, 10.0))
        .show(ui, |ui| {
            for (what, keys) in rows {
                ui.label(RichText::new(what).color(color(p.text_primary)));
                ui.label(
                    RichText::new(keys)
                        .monospace()
                        .color(color(p.text_secondary)),
                );
                ui.end_row();
            }
        });
}

fn about(ui: &mut Ui, app: &App, look: &office::Look) {
    let p = &look.palette;
    let (creator, _) = app.look(ui.ctx());
    ui.horizontal(|ui| {
        view::logo(ui, &creator, 96.0);
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.label(
                RichText::new("Hito")
                    .size(font_size::HEADLINE)
                    .strong()
                    .color(color(p.text_primary)),
            );
            ui.label(
                RichText::new(trf("Version {}", &[env!("CARGO_PKG_VERSION")]))
                    .color(color(p.text_secondary)),
            );
            ui.label(
                RichText::new(tr("Tasks, milestones and progress for your company, on a server you run yourself."))
                    .color(color(p.text_primary)),
            );
        });
    });
    office::section(ui, look, tr("Privacy"), |ui| {
        office::hint(
            ui,
            look,
            tr("Task titles, descriptions, checklists, labels, comments and milestone names are end-to-end encrypted: only your team's devices can read them. The server sees the rest: project names, who is assigned, statuses, priorities, dates and the activity log."),
        );
    });
    office::section(ui, look, tr("Credits"), |ui| {
        office::hint(ui, look, crate::icon::CREDIT);
        office::hint(
            ui,
            look,
            tr("Accent, typefaces, icons and window controls come from the Nodalia Design Language."),
        );
        office::hint(ui, look, "MIT OR Apache-2.0 · © Nodalia");
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            ui.hyperlink_to(
                "github.com/getnodalia/hito",
                "https://github.com/getnodalia/hito",
            );
        });
    });
}
