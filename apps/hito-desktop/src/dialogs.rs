//! Modal dialogs: create or join a workspace, create a channel, invite people, start a direct
//! conversation, edit a topic or a message, confirm a deletion, Preferences, About and the
//! keyboard shortcuts. Each has the NDL heading with a round close button.

use hito_core::{ChannelId, MessageId, OrganizationId, Role};
use hito_protocol::{Invite, Member, Organization, Workspace};
use egui::{vec2, Align, Id, Layout, RichText, TextEdit, Ui};
use nodalia_theme::creator::{self as cr, layout, Look};
use nodalia_theme::font_size;
use nodalia_theme::for_egui::color;

use crate::app::{Action, Main};
use crate::chrome;
use crate::i18n::{self, tr, trf, Lang};
use crate::settings::{Settings, Theme, WindowControls};
use crate::view;
use crate::widgets::avatar;

#[derive(Clone, Debug)]
pub struct OrganizationDialog {
    pub generation: u64,
    pub organizations: Vec<Organization>,
    pub selected: Option<OrganizationId>,
    pub workspaces: Vec<Workspace>,
    pub members: Vec<Member>,
    pub name: String,
    pub workspace_name: String,
    pub pending: bool,
    pub loaded: bool,
    pub details_loaded: bool,
}

#[derive(Clone, Debug)]
pub enum Dialog {
    CreateWorkspace {
        name: String,
    },
    JoinWorkspace {
        code: String,
    },
    CreateChannel {
        name: String,
        topic: String,
        private: bool,
    },
    Invite {
        invite: Option<Invite>,
        uses: u32,
        hours: u32,
    },
    NewDirect {
        filter: String,
    },
    Topic {
        channel: ChannelId,
        topic: String,
    },
    Edit {
        id: MessageId,
        body: String,
    },
    ConfirmDelete {
        id: MessageId,
    },
    Organizations(OrganizationDialog),
    Preferences,
    About,
    Shortcuts,
}

impl Dialog {
    pub fn create_channel() -> Dialog {
        Dialog::CreateChannel {
            name: String::new(),
            topic: String::new(),
            private: false,
        }
    }

    fn title(&self) -> &'static str {
        match self {
            Dialog::CreateWorkspace { .. } => tr("Create a workspace"),
            Dialog::JoinWorkspace { .. } => tr("Join a workspace"),
            Dialog::CreateChannel { .. } => tr("Create a channel"),
            Dialog::Invite { .. } => tr("Invite people"),
            Dialog::NewDirect { .. } => tr("New direct message"),
            Dialog::Topic { .. } => tr("Topic"),
            Dialog::Edit { .. } => tr("Edit message"),
            Dialog::ConfirmDelete { .. } => tr("Delete this message?"),
            Dialog::Organizations(_) => tr("Organizations"),
            Dialog::Preferences => tr("Preferences"),
            Dialog::About => tr("About Hito"),
            Dialog::Shortcuts => tr("Keyboard shortcuts"),
        }
    }
}

pub fn show(
    ctx: &egui::Context,
    main: &mut Main,
    settings: &mut Settings,
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
            ui.set_width(380.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(dialog.title())
                        .size(font_size::TITLE)
                        .strong()
                        .color(color(p.text_primary)),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
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
            if matches!(dialog, Dialog::Organizations(_)) {
                let max_height = (ctx.content_rect().height() - 140.0).max(120.0);
                egui::ScrollArea::vertical()
                    .id_salt("organization-dialog-body")
                    .max_height(max_height)
                    .show(ui, |ui| {
                        body(ui, main, settings, look, &mut dialog, actions, &mut close);
                    });
            } else {
                body(ui, main, settings, look, &mut dialog, actions, &mut close);
            }
        });
    if response.should_close() {
        close = true;
    }
    if close {
        actions.push(Action::CloseDialog);
    }
    // An action may have replaced the dialog this frame (it is applied after drawing).
    main.dialog = Some(dialog);
}

fn label(ui: &mut Ui, look: &Look, text: &str) -> egui::Response {
    ui.add_space(6.0);
    ui.label(
        RichText::new(text)
            .size(font_size::SMALL)
            .color(color(look.palette.text_secondary)),
    )
}

fn field(ui: &mut Ui, look: &Look, text: &mut String, focus: bool) -> egui::Response {
    let response = cr::field_frame(look)
        .inner_margin(egui::Margin::symmetric(8, 0))
        .show(ui, |ui| {
            ui.add(
                TextEdit::singleline(text)
                    .frame(egui::Frame::NONE)
                    .align(egui::Align2::LEFT_CENTER)
                    .desired_width(ui.available_width()),
            )
        })
        .inner;
    if focus && !response.has_focus() && ui.memory(|m| m.focused().is_none()) {
        response.request_focus();
    }
    response
}

fn buttons(ui: &mut Ui, look: &Look, confirm: &str, enabled: bool) -> (bool, bool) {
    ui.add_space(16.0);
    let mut out = (false, false);
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        out.0 = cr::primary_button(ui, look, confirm, enabled).clicked();
        out.1 = cr::secondary_button(ui, look, tr("Cancel"), true).clicked();
    });
    out
}

fn organization_role_label(role: Role) -> &'static str {
    match role {
        Role::Owner => tr("Owner"),
        Role::Admin => tr("Admin"),
        Role::Member => tr("Member"),
        Role::Guest => tr("Guest"),
    }
}

#[allow(clippy::too_many_arguments)]
fn body(
    ui: &mut Ui,
    main: &Main,
    settings: &mut Settings,
    look: &Look,
    dialog: &mut Dialog,
    actions: &mut Vec<Action>,
    close: &mut bool,
) {
    let p = &look.palette;
    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
    match dialog {
        Dialog::Organizations(state) => {
            ui.label(tr("Companies contain separate workspaces. Membership alone does not grant access to every workspace."));
            if state.pending {
                ui.label(tr("Working…"));
            }
            if !state.loaded && !state.pending {
                ui.label(tr("Could not load organizations. Close and try again."));
            }
            if state.loaded {
                egui::ScrollArea::vertical()
                    .max_height(200.0)
                    .show(ui, |ui| {
                        for organization in &state.organizations {
                            if ui
                                .add_enabled(
                                    !state.pending,
                                    egui::Button::new(&organization.name)
                                        .selected(state.selected == Some(organization.id)),
                                )
                                .clicked()
                            {
                                actions.push(Action::SelectOrganization(organization.id));
                            }
                        }
                        if state.organizations.is_empty() {
                            ui.label(tr("No organizations yet."));
                        }
                    });
                ui.add_enabled_ui(!state.pending, |ui| {
                    let name_label = label(ui, look, tr("Organization name"));
                    field(ui, look, &mut state.name, false).labelled_by(name_label.id);
                    if cr::primary_button(
                        ui,
                        look,
                        tr("Create organization"),
                        hito_core::validate::workspace_name(&state.name).is_ok(),
                    )
                    .clicked()
                    {
                        actions.push(Action::CreateOrganization(state.name.trim().to_owned()));
                    }
                });
                if let Some(org) = state
                    .organizations
                    .iter()
                    .find(|o| Some(o.id) == state.selected)
                {
                    ui.separator();
                    ui.label(RichText::new(&org.name).strong());
                    if !state.details_loaded && !state.pending {
                        ui.label(tr(
                            "Could not load organization details. Select it to retry.",
                        ));
                    }
                    if !state.details_loaded {
                        return;
                    }
                    ui.label(tr("Workspaces"));
                    egui::ScrollArea::vertical()
                        .id_salt("org-workspaces")
                        .max_height(100.0)
                        .show(ui, |ui| {
                            for workspace in &state.workspaces {
                                if ui
                                    .add_enabled(!state.pending, egui::Button::new(&workspace.name))
                                    .clicked()
                                {
                                    actions.push(Action::OpenOrganizationWorkspace(workspace.id));
                                }
                            }
                        });
                    if org.role == Role::Owner {
                        ui.add_enabled_ui(!state.pending, |ui| {
                            let name_label = label(ui, look, tr("Workspace name"));
                            field(ui, look, &mut state.workspace_name, false)
                                .labelled_by(name_label.id);
                            if cr::secondary_button(
                                ui,
                                look,
                                tr("Create workspace in organization"),
                                hito_core::validate::workspace_name(&state.workspace_name).is_ok(),
                            )
                            .clicked()
                            {
                                actions.push(Action::CreateOrganizationWorkspace(
                                    org.id,
                                    state.workspace_name.trim().to_owned(),
                                ));
                            }
                        });
                        ui.label(tr("Organization members"));
                        ui.label(tr("Only the owner can manage this organization; other organization roles have the same permissions."));
                        ui.label(tr("Changing an organization role does not change existing workspace access."));
                        egui::ScrollArea::vertical()
                            .id_salt("org-members")
                            .max_height(180.0)
                            .show(ui, |ui| {
                                for member in &state.members {
                                    ui.horizontal(|ui| {
                                        ui.label(format!(
                                            "{} (@{})",
                                            member.user.display_name, member.user.username
                                        ));
                                        if member.role == Role::Owner {
                                            ui.label(tr("Owner"));
                                        } else {
                                            let mut role = member.role;
                                            ui.add_enabled_ui(!state.pending, |ui| {
                                                egui::ComboBox::from_id_salt((
                                                    "org-role",
                                                    member.user.id.0,
                                                ))
                                                .selected_text(organization_role_label(role))
                                                .show_ui(ui, |ui| {
                                                    ui.selectable_value(
                                                        &mut role,
                                                        Role::Admin,
                                                        tr("Admin"),
                                                    );
                                                    ui.selectable_value(
                                                        &mut role,
                                                        Role::Member,
                                                        tr("Member"),
                                                    );
                                                    ui.selectable_value(
                                                        &mut role,
                                                        Role::Guest,
                                                        tr("Guest"),
                                                    );
                                                });
                                            });
                                            if role != member.role && !state.pending {
                                                actions.push(Action::SetOrganizationRole(
                                                    org.id,
                                                    member.user.id,
                                                    role,
                                                ));
                                            }
                                        }
                                    });
                                }
                            });
                    }
                }
            }
        }
        Dialog::CreateWorkspace { name } => {
            label(ui, look, tr("Workspace name"));
            field(ui, look, name, true);
            let ok = !name.trim().is_empty();
            let (confirm, cancel) = buttons(ui, look, tr("Create"), ok);
            if ok && (confirm || enter) {
                actions.push(Action::CreateWorkspace(name.trim().to_string()));
            }
            *close |= cancel;
        }
        Dialog::JoinWorkspace { code } => {
            label(ui, look, tr("Invitation code"));
            field(ui, look, code, true);
            let ok = !code.trim().is_empty();
            let (confirm, cancel) = buttons(ui, look, tr("Join"), ok);
            if ok && (confirm || enter) {
                actions.push(Action::JoinWorkspace(code.trim().to_string()));
            }
            *close |= cancel;
        }
        Dialog::CreateChannel {
            name,
            topic,
            private,
        } => {
            label(ui, look, tr("Channel name"));
            field(ui, look, name, true);
            label(ui, look, tr("Topic (optional)"));
            field(ui, look, topic, false);
            ui.add_space(8.0);
            ui.checkbox(private, tr("Private channel"));
            let ok = hito_core::validate::channel_name(name).is_ok();
            let (confirm, cancel) = buttons(ui, look, tr("Create"), ok);
            if ok && confirm {
                actions.push(Action::CreateChannel {
                    name: name.trim().to_string(),
                    topic: topic.trim().to_string(),
                    private: *private,
                });
            }
            *close |= cancel;
        }
        Dialog::Invite {
            invite,
            uses,
            hours,
        } => match invite {
            Some(invite) => {
                ui.label(
                    RichText::new(tr(
                        "Shown once. Share it privately: whoever has it can join.",
                    ))
                    .color(color(p.text_secondary)),
                );
                ui.add_space(8.0);
                cr::field_frame(look).inner_margin(10).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        RichText::new(&invite.code)
                            .monospace()
                            .size(font_size::LARGE)
                            .color(color(p.text_primary)),
                    );
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!(
                        "{}: {} · {}",
                        tr("Uses"),
                        invite.max_uses,
                        view::date(invite.expires_at)
                    ))
                    .size(font_size::CAPTION)
                    .color(color(p.text_tertiary)),
                );
                ui.add_space(12.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if cr::primary_button(ui, look, tr("Close"), true).clicked() {
                        *close = true;
                    }
                    if cr::secondary_button(ui, look, tr("Copy the code"), true).clicked() {
                        actions.push(Action::Copy(invite.code.clone()));
                    }
                });
            }
            None => {
                label(ui, look, tr("Uses"));
                ui.add(egui::Slider::new(uses, 1..=100));
                label(ui, look, tr("Expires after"));
                ui.horizontal(|ui| {
                    for (h, text) in [(24, "24 h"), (168, "7 d"), (720, "30 d")] {
                        ui.selectable_value(hours, h, text);
                    }
                });
                let (confirm, cancel) = buttons(ui, look, tr("Create"), true);
                if confirm {
                    actions.push(Action::CreateInvite {
                        max_uses: *uses,
                        hours: *hours,
                    });
                }
                *close |= cancel;
            }
        },
        Dialog::NewDirect { filter } => {
            field(ui, look, filter, true);
            ui.add_space(8.0);
            let needle = filter.trim().to_lowercase();
            let members = main
                .model
                .workspace
                .and_then(|w| main.model.members.get(&w))
                .cloned()
                .unwrap_or_default();
            let others: Vec<_> = members
                .into_iter()
                .filter(|m| m.user.id != main.model.me.id)
                .filter(|m| {
                    needle.is_empty()
                        || m.user.username.contains(&needle)
                        || m.user.display_name.to_lowercase().contains(&needle)
                })
                .collect();
            if others.is_empty() {
                ui.label(
                    RichText::new(tr("Nobody else is here yet. Invite someone first."))
                        .color(color(p.text_secondary)),
                );
            }
            egui::ScrollArea::vertical()
                .max_height(300.0)
                .show(ui, |ui| {
                    for m in others {
                        ui.horizontal(|ui| {
                            avatar(
                                ui,
                                look,
                                m.user.id.0,
                                &m.user.display_name,
                                24.0,
                                Some(main.model.online.contains(&m.user.id)),
                            );
                            if ui
                                .selectable_label(
                                    false,
                                    format!("{}  @{}", m.user.display_name, m.user.username),
                                )
                                .clicked()
                            {
                                actions.push(Action::OpenDirect(m.user.id));
                            }
                        });
                    }
                });
        }
        Dialog::Topic { channel, topic } => {
            field(ui, look, topic, true);
            let (confirm, cancel) = buttons(ui, look, tr("Save"), true);
            if confirm || enter {
                actions.push(Action::UpdateTopic(*channel, topic.trim().to_string()));
            }
            *close |= cancel;
        }
        Dialog::Edit { id, body } => {
            cr::field_frame(look).inner_margin(8).show(ui, |ui| {
                ui.add(
                    TextEdit::multiline(body)
                        .frame(egui::Frame::NONE)
                        .desired_rows(4)
                        .desired_width(ui.available_width()),
                );
            });
            let ok = !body.trim().is_empty();
            let (confirm, cancel) = buttons(ui, look, tr("Save"), ok);
            if ok && confirm {
                actions.push(Action::SaveEdit(*id, body.trim_end().to_string()));
            }
            *close |= cancel;
        }
        Dialog::ConfirmDelete { id } => {
            let (confirm, cancel) = buttons(ui, look, tr("Delete"), true);
            if confirm {
                actions.push(Action::Delete(*id));
            }
            *close |= cancel;
        }
        Dialog::Preferences => preferences(ui, settings, look, actions),
        Dialog::About => about(ui, look),
        Dialog::Shortcuts => shortcuts(ui, look),
    }
}

fn preferences(ui: &mut Ui, settings: &mut Settings, look: &Look, actions: &mut Vec<Action>) {
    let p = &look.palette;
    let before = settings.clone();
    let heading = |ui: &mut Ui, t: &str| {
        ui.add_space(10.0);
        ui.label(RichText::new(t).strong().color(color(p.text_primary)));
    };
    heading(ui, tr("Appearance"));
    label(ui, look, tr("Theme"));
    ui.horizontal(|ui| {
        ui.selectable_value(&mut settings.theme, Theme::System, tr("System"));
        ui.selectable_value(&mut settings.theme, Theme::Light, tr("Light"));
        ui.selectable_value(&mut settings.theme, Theme::Dark, tr("Dark"));
    });
    ui.checkbox(&mut settings.high_contrast, tr("High contrast"));
    ui.checkbox(&mut settings.reduced_motion, tr("Reduce motion"));
    if chrome::NDL_CONTROLS_POSSIBLE {
        heading(ui, tr("Window"));
        label(ui, look, tr("The window buttons on Linux."));
        ui.horizontal(|ui| {
            ui.selectable_value(
                &mut settings.window_controls,
                WindowControls::Nodalia,
                tr("Nodalia controls"),
            );
            ui.selectable_value(
                &mut settings.window_controls,
                WindowControls::System,
                tr("System controls"),
            );
        });
    }
    heading(ui, tr("Language"));
    ui.horizontal(|ui| {
        ui.selectable_value(&mut settings.language, None, tr("System"));
        ui.selectable_value(&mut settings.language, Some(Lang::Es), Lang::Es.name());
        ui.selectable_value(&mut settings.language, Some(Lang::En), Lang::En.name());
    });
    heading(ui, tr("Account"));
    ui.checkbox(
        &mut settings.remember,
        tr("Keep me signed in on this computer"),
    );
    ui.checkbox(&mut settings.context_panel, tr("The details panel"));
    #[cfg(target_os = "linux")]
    {
        heading(ui, tr("Notifications"));
        ui.checkbox(
            &mut settings.notifications,
            tr("Notify me about new messages"),
        );
        ui.label(tr("Notifications never include message text or sender."));
    }
    if *settings != before {
        actions.push(Action::SettingsChanged);
    }
}

fn about(ui: &mut Ui, look: &Look) {
    let p = &look.palette;
    ui.vertical_centered(|ui| {
        view::logo(ui, look, 96.0);
        ui.add_space(8.0);
        ui.label(RichText::new("Hito").size(font_size::HEADLINE).strong().color(color(p.text_primary)));
        ui.label(RichText::new(trf("Version {}", &[env!("CARGO_PKG_VERSION")])).color(color(p.text_secondary)));
        ui.add_space(8.0);
        ui.label(RichText::new(tr("Open source, self-hosted team chat.")).color(color(p.text_primary)));
        ui.label(RichText::new(tr("Everything that happens here stays on your server.")).color(color(p.text_secondary)));
        ui.add_space(8.0);
        ui.label(
            RichText::new(tr("End-to-end encryption is not available. Use TLS (https) for any server outside your computer."))
                .size(font_size::SMALL)
                .color(color(p.status_warning_fg)),
        );
        ui.add_space(8.0);
        ui.label(RichText::new(tr("Accent, typefaces, icons and window controls come from the Nodalia Design Language.")).size(font_size::SMALL).color(color(p.text_secondary)));
        ui.add_space(8.0);
        ui.label(RichText::new("MIT OR Apache-2.0 · © Nodalia").size(font_size::SMALL).color(color(p.text_secondary)));
        ui.label(RichText::new(crate::icon::CREDIT).size(font_size::CAPTION).color(color(p.text_tertiary)));
        ui.hyperlink_to("github.com/getnodalia/hito", "https://github.com/getnodalia/hito");
    });
    let _ = i18n::current();
}

fn shortcuts(ui: &mut Ui, look: &Look) {
    let p = &look.palette;
    let command = if cfg!(target_os = "macos") {
        "⌘"
    } else {
        "Ctrl"
    };
    let rows = [
        (tr("Send"), "Enter".to_string()),
        (tr("New line"), "Shift+Enter".to_string()),
        (tr("Search"), format!("{command}+K")),
        (tr("Move between channels"), "Alt+↑ / Alt+↓".to_string()),
        (tr("Close the thread"), "Esc".to_string()),
        (tr("Preferences"), format!("{command}+,")),
    ];
    egui::Grid::new("shortcuts")
        .num_columns(2)
        .spacing(vec2(24.0, 8.0))
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
