//! The first screen: which server, and sign in or create an account.

use egui::{Align, Key, Layout, RichText, TextEdit, Ui, UiBuilder};
use hito_protocol::Registration;
use nodalia_theme::creator::{self as cr, layout, Look};
use nodalia_theme::font_size;
use nodalia_theme::for_egui::color;

use crate::app::{Action, App, Screen, ServerCheck};
use crate::i18n::tr;
use crate::settings::Settings;
use crate::{chrome, view};

pub struct Form {
    pub server: String,
    pub username: String,
    pub display_name: String,
    pub password: String,
    pub invite: String,
    pub register: bool,
    pub busy: bool,
    pub error: Option<String>,
    /// A message from before this screen (signed out elsewhere).
    pub notice: Option<String>,
    pub info: Option<ServerCheck>,
    /// The address `info` belongs to.
    pub info_for: String,
}

impl Form {
    pub fn new(settings: &Settings) -> Form {
        Form {
            server: settings.server.clone(),
            username: settings.username.clone(),
            display_name: String::new(),
            password: String::new(),
            invite: String::new(),
            register: false,
            busy: false,
            error: None,
            notice: None,
            info: None,
            info_for: String::new(),
        }
    }
}

pub fn show(ui: &mut Ui, app: &mut App) -> Vec<Action> {
    let mut actions = Vec::new();
    let ctx = ui.ctx().clone();
    let (look, mode) = app.look(&ctx);
    let full = ui.max_rect();
    ui.painter()
        .rect_filled(full, 0.0, color(look.palette.surface_window));

    let bar = full.with_max_y(full.top() + layout::APP_BAR);
    let ndl = app.ndl_controls();
    if chrome::drags_window(ndl) {
        chrome::drag_area(ui, bar);
    }
    if ndl {
        chrome::controls(ui, bar, &look.palette, mode);
    }

    let mut remember = app.settings.remember;
    let card_width = 380.0;
    let tall = matches!(&app.screen, Screen::SignIn(form) if form.register);
    let card_height = if tall { 720.0 } else { 600.0 };
    let body = egui::Rect::from_min_max(egui::pos2(full.left(), bar.bottom()), full.max);
    ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let side = ((ui.available_width() - card_width) / 2.0).max(8.0);
                ui.add_space(((body.height() - card_height) / 2.0).max(8.0));
                ui.horizontal(|ui| {
                    ui.add_space(side);
                    ui.vertical(|ui| match &mut app.screen {
                        Screen::Restoring => {
                            cr::island(&look).inner_margin(24).show(ui, |ui| {
                                ui.set_width(card_width - 48.0);
                                ui.vertical_centered(|ui| {
                                    view::logo(ui, &look, 64.0);
                                    ui.add_space(12.0);
                                    ui.spinner();
                                    ui.label(
                                        RichText::new(tr("Connecting…"))
                                            .color(color(look.palette.text_secondary)),
                                    );
                                });
                            });
                        }
                        Screen::SignIn(form) => {
                            form_card(ui, &look, form, &mut remember, card_width, &mut actions);
                        }
                        Screen::Main(_) => {}
                    });
                });
                ui.add_space(16.0);
            });
    });
    if remember != app.settings.remember {
        app.settings.remember = remember;
        actions.push(Action::SettingsChanged);
    }
    view::toasts(ui, app, &look);
    if ndl {
        chrome::resize_edges(ui, full);
    }
    actions
}

fn form_card(
    ui: &mut Ui,
    look: &Look,
    form: &mut Form,
    remember: &mut bool,
    width: f32,
    actions: &mut Vec<Action>,
) {
    let p = &look.palette;
    cr::island(look).inner_margin(24).show(ui, |ui| {
        ui.set_width(width - 48.0);
        ui.vertical_centered(|ui| {
            view::logo(ui, look, 72.0);
            ui.add_space(8.0);
            let title = if form.register {
                tr("Create an account")
            } else {
                tr("Sign in to Hito")
            };
            ui.label(
                RichText::new(title)
                    .size(font_size::TITLE)
                    .strong()
                    .color(color(p.text_primary)),
            );
            ui.label(
                RichText::new(tr("Tasks, milestones and progress for your company, on a server you run yourself."))
                    .size(font_size::SMALL)
                    .color(color(p.text_secondary)),
            );
        });
        ui.add_space(16.0);

        if let Some(notice) = &form.notice {
            status_line(ui, look, notice, false);
        }

        field_label(ui, look, tr("Server"));
        let server = field(ui, look, &mut form.server, "chat.example.org", false);
        if server.lost_focus() {
            actions.push(Action::CheckServer);
        }
        server_status(ui, look, form);

        if form.register {
            field_label(ui, look, tr("Display name"));
            field(ui, look, &mut form.display_name, "", false);
        }
        field_label(ui, look, tr("Username"));
        field(ui, look, &mut form.username, "", false);
        field_label(ui, look, tr("Password"));
        let password = field(ui, look, &mut form.password, "", true);
        if form.register {
            field_label(ui, look, tr("Invitation code (if you have one)"));
            field(ui, look, &mut form.invite, "", false);
        }
        ui.add_space(6.0);
        ui.checkbox(remember, tr("Keep me signed in on this computer"));

        if let Some(error) = &form.error {
            ui.add_space(6.0);
            status_line(ui, look, error, true);
        }
        ui.add_space(12.0);

        let ready = !form.busy
            && !form.server.trim().is_empty()
            && !form.username.trim().is_empty()
            && !form.password.is_empty()
            && (!form.register || !form.display_name.trim().is_empty());
        let label = if form.register {
            tr("Create account")
        } else {
            tr("Sign in")
        };
        let submitted = password.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        ui.with_layout(Layout::top_down(Align::Center), |ui| {
            let button = cr::primary_button_min(ui, look, label, ready, width - 48.0);
            if (button.clicked() || submitted) && ready {
                actions.push(Action::SignIn);
            }
            if form.busy {
                ui.add_space(6.0);
                ui.spinner();
            }
            ui.add_space(8.0);
            let switch = if form.register {
                tr("Back to sign in")
            } else {
                tr("Create an account")
            };
            if ui
                .link(RichText::new(switch).color(color(p.accent_text)))
                .clicked()
            {
                form.register = !form.register;
                form.error = None;
            }
        });
    });
}

fn server_status(ui: &mut Ui, look: &Look, form: &Form) {
    let p = &look.palette;
    if let Ok(url) = hito_client::parse_server_url(&form.server) {
        if hito_client::is_insecure(&url) {
            status_line(
                ui,
                look,
                tr("This address uses http: passwords and everything the server sees travel unencrypted. Use it only on your own computer or network."),
                true,
            );
        }
    }
    match &form.info {
        Some(Ok(info)) => {
            let policy = match info.registration {
                Registration::Open => tr("Anyone can create an account."),
                Registration::InviteOnly => tr("Only people with an invitation can register."),
                Registration::Closed => tr("Registration is closed on this server."),
            };
            ui.label(
                RichText::new(format!("{} {} · {}", info.name, info.version, policy))
                    .size(font_size::CAPTION)
                    .color(color(p.text_secondary)),
            );
        }
        Some(Err(e)) => status_line(ui, look, e, true),
        None => {}
    }
}

fn field_label(ui: &mut Ui, look: &Look, text: &str) {
    ui.add_space(6.0);
    ui.label(
        RichText::new(text)
            .size(font_size::SMALL)
            .color(color(look.palette.text_secondary)),
    );
}

fn field(
    ui: &mut Ui,
    look: &Look,
    text: &mut String,
    hint: &str,
    password: bool,
) -> egui::Response {
    cr::field_frame(look)
        .inner_margin(egui::Margin::symmetric(8, 0))
        .show(ui, |ui| {
            ui.add(
                TextEdit::singleline(text)
                    .frame(egui::Frame::NONE)
                    .align(egui::Align2::LEFT_CENTER)
                    .hint_text(hint)
                    .password(password)
                    .desired_width(ui.available_width()),
            )
        })
        .inner
}

pub fn status_line(ui: &mut Ui, look: &Look, text: &str, danger: bool) {
    let p = &look.palette;
    let (fg, bg) = if danger {
        (p.status_danger_fg, p.status_danger_bg)
    } else {
        (p.status_info_fg, p.status_info_bg)
    };
    egui::Frame::NONE
        .fill(color(bg))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(text).size(font_size::SMALL).color(color(fg)));
        });
}
