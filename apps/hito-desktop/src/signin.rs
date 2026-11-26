//! The first screen: which server, and sign in or create an account.

use egui::{Align, Key, Layout, RichText, Ui};
use hito_protocol::Registration;
use nodalia_theme::creator::{self as cr, Look};
use nodalia_theme::enterprise::{self as ent, field, field_label, status_line};
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
    let full = ent::window_surface(ui, &look);

    let bar = full.with_max_y(full.top() + ent::layout::APP_BAR);
    let ndl = app.ndl_controls();
    if chrome::drags_window(ndl) {
        chrome::drag_area(ui, bar);
    }
    if ndl {
        chrome::controls(ui, bar, &look.palette, mode);
    }

    let mut remember = app.settings.remember;
    let tall = matches!(&app.screen, Screen::SignIn(form) if form.register);
    let card_height = if tall { 720.0 } else { 600.0 };
    let body = egui::Rect::from_min_max(egui::pos2(full.left(), bar.bottom()), full.max);
    ent::sign_in_page(ui, body, card_height, |ui| match &mut app.screen {
        Screen::Restoring => {
            ent::sign_in_card(ui, &look, |ui| {
                ui.vertical_centered(|ui| {
                    view::logo(ui, &look, 64.0);
                    ui.add_space(12.0);
                    ui.spinner();
                    ui.label(
                        RichText::new(tr("Connecting…")).color(color(look.palette.text_secondary)),
                    );
                });
            });
        }
        Screen::SignIn(form) => {
            form_card(ui, &look, form, &mut remember, &mut actions);
        }
        Screen::Main(_) => {}
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
    actions: &mut Vec<Action>,
) {
    let p = &look.palette;
    ent::sign_in_card(ui, look, |ui| {
        let title = if form.register {
            tr("Create an account")
        } else {
            tr("Sign in to Hito")
        };
        ent::sign_in_head(
            ui,
            look,
            &crate::icon::texture(ui.ctx()),
            title,
            tr("Tasks, milestones and progress for your company, on a server you run yourself."),
        );

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
            let button =
                cr::primary_button_min(ui, look, label, ready, ent::layout::SIGN_IN - 48.0);
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
