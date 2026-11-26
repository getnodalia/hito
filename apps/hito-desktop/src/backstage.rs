//! The Home view, drawn over the whole window: start a project from a template, recent projects.
//! It sits under the Enterprise top bar (`nodalia_theme::enterprise`); Esc or Back returns to the
//! work. Preferences, Keyboard shortcuts and About are dialogs (`dialogs.rs`).

use egui::{pos2, Key, Rect, RichText, Stroke, Ui};
use nodalia_theme::enterprise::{self as ent, Panes};
use nodalia_theme::font_size;
use nodalia_theme::for_egui::color;
use nodalia_theme::office;
use nodalia_theme::window_controls;

use crate::app::{Action, App, Screen};
use crate::dates;
use crate::dialogs::Dialog;
use crate::i18n::{tr, trf};
use crate::model::Page;
use crate::templates::Kind;

pub fn show(ui: &mut Ui, app: &mut App) -> Vec<Action> {
    let mut actions = Vec::new();
    let ctx = ui.ctx().clone();
    let (look, mode) = app.look(&ctx);
    let ndl = app.ndl_controls();
    let full = ui.max_rect();
    ui.painter()
        .rect_filled(full, 0.0, color(look.palette.surface_window));

    let Screen::Main(main) = &app.screen else {
        return actions;
    };
    if main.backstage.is_none() {
        return actions;
    }

    let panes = Panes::new(full, None);
    let labels = window_controls::Labels {
        minimize: tr("Minimize"),
        maximize: tr("Maximize"),
        restore: tr("Restore"),
        close: tr("Close"),
    };
    ent::top_bar(ui, &look, mode, ndl, panes.bar, labels, |ui| {
        if ui.button(tr("Back")).clicked() {
            actions.push(Action::Backstage(None));
        }
        ent::brand(ui, &look, &crate::icon::texture(ui.ctx()), "Hito");
    });

    let typing = ctx.memory(|m| m.focused().is_some());
    if !typing && ctx.input(|i| i.key_pressed(Key::Escape)) {
        actions.push(Action::Backstage(None));
    }

    let office_look = office::Look::new(look.palette);
    let gap = ent::layout::GAP;
    let page = Rect::from_min_max(
        pos2(full.left() + gap, panes.bar.bottom()),
        pos2(full.right() - gap, full.bottom() - gap),
    );
    ent::island(ui, &look, page, "home", |ui| {
        let inner = ui.max_rect().shrink2(egui::vec2(48.0, 24.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
            office::page(ui, &office_look, tr("Home"), |ui| {
                home(ui, app, &office_look, &mut actions);
            });
        });
    });
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
