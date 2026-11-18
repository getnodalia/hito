//! The signed-in window: the top bar, and three islands. Left: workspace, channels and direct
//! messages. Centre: the conversation and the composer. Right (optional): details, a thread or
//! search results.

use std::sync::Arc;

use hito_core::{ChannelId, ChannelKind, MessageId, UserId};
use hito_protocol::{Channel, Message, Reaction, Timestamp};
use egui::{
    pos2, vec2, Align, Color32, CornerRadius, Id, Key, Layout, Margin, Modifiers, Rect, RichText,
    Sense, Stroke, TextEdit, Ui, UiBuilder,
};
use nodalia_theme::creator::{self as cr, layout, Look};
use nodalia_theme::for_egui::color;
use nodalia_theme::{font_size, Mode};

use crate::app::{Action, App, Main, Panel, Screen};
use crate::chrome;
use crate::dialogs::{self, Dialog};
use crate::i18n::{tr, trf};
use crate::model::{peer_name, Connection, Model, Pending};
use crate::widgets::{self, avatar, badge, icon, icon_button};

const SIDEBAR: f32 = 260.0;
const CONTEXT: f32 = 300.0;
/// Messages by the same person closer than this share one header.
const GROUP_MS: i64 = 5 * 60 * 1000;
pub const QUICK_REACTIONS: [&str; 8] = ["👍", "❤", "😂", "🎉", "👀", "🙏", "✅", "🔥"];

pub fn show(ui: &mut Ui, app: &mut App, now: f64) -> Vec<Action> {
    let mut actions = Vec::new();
    let ctx = ui.ctx().clone();
    let (look, mode) = app.look(&ctx);
    let ndl = app.ndl_controls();
    let panel_open = app.settings.context_panel;
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
        top_bar(ui, bar, main, &look, mode, ndl, panel_open, &mut actions);

        let gap = layout::GAP;
        let body = Rect::from_min_max(
            pos2(full.left() + gap, bar.bottom()),
            pos2(full.right() - gap, full.bottom() - gap),
        );
        let side = Rect::from_min_max(body.min, pos2(body.left() + SIDEBAR, body.bottom()));
        let show_context = panel_open && body.width() > SIDEBAR + CONTEXT + 360.0;
        let right = Rect::from_min_max(pos2(body.right() - CONTEXT, body.top()), body.max);
        let center_right = if show_context {
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
        island(ui, &look, center, "center", |ui| {
            conversation(ui, main, &look, now, &mut actions)
        });
        if show_context {
            island(ui, &look, right, "context", |ui| {
                context_panel(ui, main, &look, now, &mut actions)
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

fn island(ui: &mut Ui, look: &Look, rect: Rect, id: &str, add: impl FnOnce(&mut Ui)) {
    ui.scope_builder(UiBuilder::new().max_rect(rect).id_salt(id), |ui| {
        cr::island(look).show(ui, |ui| {
            ui.set_min_size(rect.size());
            ui.set_max_size(rect.size());
            add(ui);
        });
    });
}

fn shortcuts(ctx: &egui::Context, main: &mut Main, actions: &mut Vec<Action>) {
    let (search, prefs, up, down, escape) = ctx.input_mut(|i| {
        (
            i.consume_key(Modifiers::COMMAND, Key::K),
            i.consume_key(Modifiers::COMMAND, Key::Comma),
            i.consume_key(Modifiers::ALT, Key::ArrowUp),
            i.consume_key(Modifiers::ALT, Key::ArrowDown),
            i.key_pressed(Key::Escape),
        )
    });
    if search {
        main.focus_search = true;
    }
    if prefs {
        actions.push(Action::Dialog(Dialog::Preferences));
    }
    if up || down {
        let (rooms, direct) = main.model.sidebar();
        let order: Vec<ChannelId> = rooms.iter().chain(direct.iter()).map(|c| c.id).collect();
        let at = main
            .model
            .channel
            .and_then(|id| order.iter().position(|c| *c == id));
        let next = match (at, up) {
            (Some(i), true) => i.checked_sub(1),
            (Some(i), false) => Some(i + 1),
            (None, _) => Some(0),
        };
        if let Some(id) = next.and_then(|i| order.get(i)) {
            actions.push(Action::SelectChannel(*id));
        }
    }
    if escape && main.dialog.is_none() && main.panel != Panel::Details {
        actions.push(Action::ClosePanel);
    }
}

// --- Top bar -----------------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn top_bar(
    ui: &mut Ui,
    bar: Rect,
    main: &mut Main,
    look: &Look,
    mode: Mode,
    ndl: bool,
    panel_open: bool,
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
                ui.set_min_width(200.0);
                if ui.button(tr("Preferences")).clicked() {
                    actions.push(Action::Dialog(Dialog::Preferences));
                }
                if ui.button(tr("Keyboard shortcuts")).clicked() {
                    actions.push(Action::Dialog(Dialog::Shortcuts));
                }
                if ui.button(tr("About Hito")).clicked() {
                    actions.push(Action::Dialog(Dialog::About));
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
            let width = (rest - 220.0).clamp(160.0, 420.0);
            ui.add_space(((rest - width) / 2.0 - 110.0).max(8.0));
            let response = crate::widgets::search_field(
                ui,
                look,
                &mut main.search,
                tr("Search messages"),
                width,
            );
            if main.focus_search {
                response.request_focus();
                main.focus_search = false;
            }
            if response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                actions.push(Action::Search);
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let toggle = icon_button(
                    ui,
                    look,
                    "panel-right",
                    tr("Show or hide the details panel"),
                    panel_open,
                );
                if toggle.clicked() {
                    actions.push(Action::TogglePanel);
                }
                ui.add_space(4.0);
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
            workspace_switcher(ui, &main.model, look, actions);
            ui.add_space(6.0);
            let Some(ws) = main.model.current_workspace().cloned() else {
                return;
            };
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let (rooms, direct) = main.model.sidebar();
                    section(
                        ui,
                        look,
                        tr("Channels"),
                        ws.role.can_create_channel(),
                        tr("Create a channel"),
                        || {
                            actions.push(Action::Dialog(Dialog::create_channel()));
                        },
                    );
                    for c in rooms {
                        let kind = if c.kind == ChannelKind::Private {
                            "lock"
                        } else {
                            "hash"
                        };
                        let r = channel_row(
                            ui,
                            look,
                            RowIcon::Icon(kind),
                            &c.name,
                            main.model.channel == Some(c.id),
                            c.unread,
                            c.mentions,
                            false,
                        );
                        if r.clicked() {
                            actions.push(Action::SelectChannel(c.id));
                        }
                    }
                    for c in main.model.browsable() {
                        if main.model.channel == Some(c.id) {
                            continue;
                        }
                        let r = channel_row(
                            ui,
                            look,
                            RowIcon::Icon("hash"),
                            &c.name,
                            false,
                            0,
                            0,
                            true,
                        );
                        if r.clicked() {
                            actions.push(Action::SelectChannel(c.id));
                        }
                    }
                    ui.add_space(10.0);
                    section(
                        ui,
                        look,
                        tr("Direct messages"),
                        true,
                        tr("New direct message"),
                        || {
                            actions.push(Action::Dialog(Dialog::NewDirect {
                                filter: String::new(),
                            }));
                        },
                    );
                    for c in direct {
                        let peer = c.peer.as_ref();
                        let online = peer.map(|u| main.model.online.contains(&u.id));
                        let seed = peer.map_or(c.id.0, |u| u.id.0);
                        let name = peer_name(c);
                        let r = channel_row(
                            ui,
                            look,
                            RowIcon::Avatar(seed, online),
                            &name,
                            main.model.channel == Some(c.id),
                            c.unread,
                            c.mentions,
                            false,
                        );
                        if r.clicked() {
                            actions.push(Action::SelectChannel(c.id));
                        }
                    }
                });
        },
    );

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
            let online = main.model.connection == Connection::Connected;
            avatar(ui, look, me.id.0, &me.display_name, 30.0, Some(online));
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(&me.display_name)
                        .strong()
                        .color(color(p.text_primary)),
                );
                let state = if online { tr("Online") } else { tr("Offline") };
                ui.label(
                    RichText::new(format!("@{} · {state}", me.username))
                        .size(font_size::CAPTION)
                        .color(color(p.text_secondary)),
                );
            });
        },
    );
}

fn workspace_switcher(ui: &mut Ui, model: &Model, look: &Look, actions: &mut Vec<Action>) {
    let p = &look.palette;
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
            let (unread, mentions) = model.workspace_unread(w.id);
            ui.horizontal(|ui| {
                let selected = model.workspace == Some(w.id);
                if ui.selectable_label(selected, &w.name).clicked() {
                    actions.push(Action::SelectWorkspace(w.id));
                }
                if mentions > 0 {
                    badge(ui, look, mentions, true);
                } else if unread > 0 {
                    badge(ui, look, unread, false);
                }
            });
        }
        ui.separator();
        if ui.button(tr("Organizations")).clicked() {
            actions.push(Action::OpenOrganizations);
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

fn section(
    ui: &mut Ui,
    look: &Look,
    title: &str,
    can_add: bool,
    add_label: &str,
    on_add: impl FnOnce(),
) {
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        ui.label(
            RichText::new(title)
                .size(font_size::SMALL)
                .strong()
                .color(color(look.palette.text_secondary)),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if can_add && icon_button(ui, look, "plus", add_label, false).clicked() {
                on_add();
            }
        });
    });
}

enum RowIcon {
    Icon(&'static str),
    Avatar(i64, Option<bool>),
}

#[allow(clippy::too_many_arguments)]
fn channel_row(
    ui: &mut Ui,
    look: &Look,
    kind: RowIcon,
    name: &str,
    selected: bool,
    unread: u32,
    mentions: u32,
    muted: bool,
) -> egui::Response {
    let p = &look.palette;
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
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
    let strong = unread > 0 && !selected;
    let ink = if selected {
        color(p.accent_text)
    } else if muted {
        color(p.text_tertiary)
    } else if strong {
        color(p.text_primary)
    } else {
        color(p.text_secondary)
    };
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink2(vec2(8.0, 0.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    match kind {
        RowIcon::Icon(name) => {
            icon(&mut child, name, 16.0, ink);
        }
        RowIcon::Avatar(seed, online) => avatar(&mut child, look, seed, name, 20.0, online),
    }
    child.add_space(6.0);
    let mut text = RichText::new(name).color(ink);
    if strong || selected {
        text = text.strong();
    }
    child.add(egui::Label::new(text).truncate().selectable(false));
    child.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if mentions > 0 {
            badge(ui, look, mentions, true);
        } else if unread > 0 && !selected {
            badge(ui, look, unread, false);
        }
    });
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, name));
    response
}

// --- Conversation ------------------------------------------------------------------------------

fn conversation(ui: &mut Ui, main: &mut Main, look: &Look, now: f64, actions: &mut Vec<Action>) {
    let p = &look.palette;
    if main.model.workspaces.is_empty() {
        empty_state(
            ui,
            look,
            tr("You don't belong to any workspace yet."),
            |ui| {
                if cr::primary_button_min(ui, look, tr("Create a workspace"), true, 220.0).clicked()
                {
                    actions.push(Action::Dialog(Dialog::CreateWorkspace {
                        name: String::new(),
                    }));
                }
                ui.add_space(6.0);
                if cr::secondary_button_min(ui, look, tr("Join a workspace"), true, 220.0).clicked()
                {
                    actions.push(Action::Dialog(Dialog::JoinWorkspace {
                        code: String::new(),
                    }));
                }
            },
        );
        return;
    }
    let Some(channel) = main.model.current_channel().cloned() else {
        empty_state(ui, look, tr("No channel selected"), |_| {});
        return;
    };
    let role = main.model.current_workspace().map(|w| w.role);

    // Header.
    ui.scope_builder(
        UiBuilder::new().max_rect(ui.max_rect().shrink2(vec2(16.0, 0.0))),
        |ui| {
            ui.set_height(56.0);
            ui.horizontal_centered(|ui| {
                let direct = channel.kind == ChannelKind::Direct;
                if direct {
                    let peer = channel.peer.as_ref();
                    avatar(
                        ui,
                        look,
                        peer.map_or(0, |u| u.id.0),
                        &peer_name(&channel),
                        28.0,
                        peer.map(|u| main.model.online.contains(&u.id)),
                    );
                } else {
                    icon(
                        ui,
                        if channel.kind == ChannelKind::Private {
                            "lock"
                        } else {
                            "hash"
                        },
                        20.0,
                        color(p.text_secondary),
                    );
                }
                ui.vertical(|ui| {
                    // The icon is centred in the 56-point header. Leave room for the
                    // smaller topic line when present and centre a single-line title.
                    ui.add_space(if channel.topic.is_empty() { 16.0 } else { 4.0 });
                    let title = if direct {
                        peer_name(&channel)
                    } else {
                        channel.name.clone()
                    };
                    ui.label(
                        RichText::new(title)
                            .size(font_size::TITLE)
                            .strong()
                            .color(color(p.text_primary)),
                    );
                    if !channel.topic.is_empty() {
                        ui.add(
                            egui::Label::new(
                                RichText::new(&channel.topic)
                                    .size(font_size::SMALL)
                                    .color(color(p.text_secondary)),
                            )
                            .truncate(),
                        );
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if !channel.joined && channel.kind == ChannelKind::Public {
                        if cr::primary_button(ui, look, tr("Join channel"), true).clicked() {
                            actions.push(Action::JoinChannel(channel.id));
                        }
                    } else if role.is_some_and(|r| r.can_invite())
                        && !direct
                        && cr::secondary_button(ui, look, tr("Invite people"), true).clicked()
                    {
                        actions.push(Action::Dialog(Dialog::Invite {
                            invite: None,
                            uses: 5,
                            hours: 168,
                        }));
                    }
                    if !direct {
                        ui.label(
                            RichText::new(if channel.member_count == 1 {
                                tr("1 member").to_string()
                            } else {
                                trf("{} members", &[&channel.member_count.to_string()])
                            })
                            .size(font_size::SMALL)
                            .color(color(p.text_secondary)),
                        );
                    }
                });
            });
        },
    );
    let line_y = ui.min_rect().bottom();
    ui.painter().line_segment(
        [
            pos2(ui.max_rect().left(), line_y),
            pos2(ui.max_rect().right(), line_y),
        ],
        Stroke::new(1.0, color(p.border_subtle)),
    );

    let hint = if channel.kind == ChannelKind::Direct {
        trf("Message {}", &[&peer_name(&channel)])
    } else {
        trf("Message #{}", &[&channel.name])
    };
    ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
        ui.add_space(12.0);
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(ui.max_rect().shrink2(vec2(16.0, 0.0)))
                .layout(Layout::bottom_up(Align::Min)),
            |ui| {
                if channel.joined || channel.kind != ChannelKind::Public {
                    composer(ui, main, look, channel.id, None, &hint, actions);
                } else {
                    signin_hint(ui, look, tr("You are not a member of this channel."));
                }
                typing_line(ui, look, &main.model, channel.id, None, now);
            },
        );
        ui.with_layout(Layout::top_down(Align::Min), |ui| {
            history(ui, main, look, &channel, actions);
        });
    });
}

fn signin_hint(ui: &mut Ui, look: &Look, text: &str) {
    ui.label(RichText::new(text).color(color(look.palette.text_secondary)));
}

fn empty_state(ui: &mut Ui, look: &Look, text: &str, extra: impl FnOnce(&mut Ui)) {
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() / 2.0 - 80.0).max(16.0));
        logo(ui, look, 56.0);
        ui.add_space(12.0);
        ui.label(
            RichText::new(text)
                .size(font_size::LARGE)
                .color(color(look.palette.text_secondary)),
        );
        ui.add_space(12.0);
        extra(ui);
    });
}

fn history(
    ui: &mut Ui,
    main: &mut Main,
    look: &Look,
    channel: &Channel,
    actions: &mut Vec<Action>,
) {
    let p = &look.palette;
    let Some(conv) = main.model.conversations.get(&channel.id) else {
        return;
    };
    if !conv.loaded {
        ui.vertical_centered(|ui| {
            ui.add_space(40.0);
            ui.spinner();
            ui.label(RichText::new(tr("Loading…")).color(color(p.text_secondary)));
        });
        return;
    }
    let messages = conv.messages.clone();
    let has_more = conv.has_more;
    let loading = conv.loading;
    let first_unread = conv.first_unread;
    let pending: Vec<Pending> = main
        .model
        .pending
        .iter()
        .filter(|x| x.channel == channel.id && x.thread_root.is_none())
        .cloned()
        .collect();
    egui::ScrollArea::vertical()
        .id_salt(("history", channel.id.0))
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            ui.add_space(8.0);
            if has_more {
                ui.vertical_centered(|ui| {
                    if loading {
                        ui.spinner();
                    } else if cr::secondary_button(ui, look, tr("Load older messages"), true)
                        .clicked()
                    {
                        actions.push(Action::LoadOlder(channel.id));
                    }
                });
            } else {
                let start = if channel.kind == ChannelKind::Direct {
                    trf(
                        "This is the start of your conversation with {}.",
                        &[&peer_name(channel)],
                    )
                } else {
                    trf("This is the start of #{}.", &[&channel.name])
                };
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    ui.label(RichText::new(start).color(color(p.text_secondary)));
                });
                if messages.is_empty() && pending.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(16.0);
                        ui.label(
                            RichText::new(tr("No messages yet. Say hello."))
                                .color(color(p.text_tertiary)),
                        );
                    });
                }
            }
            message_list(ui, main, look, &messages, first_unread, true, actions);
            pending_list(ui, look, &main.model, &pending, actions);
            ui.add_space(8.0);
        });
}

fn message_list(
    ui: &mut Ui,
    main: &mut Main,
    look: &Look,
    messages: &[Message],
    first_unread: Option<MessageId>,
    threads: bool,
    actions: &mut Vec<Action>,
) {
    let mut previous: Option<&Message> = None;
    for message in messages {
        if let Some(prev) = previous {
            if day(prev.created_at) != day(message.created_at) {
                day_separator(ui, look, message.created_at);
            }
        } else {
            day_separator(ui, look, message.created_at);
        }
        let unread_line = first_unread == Some(message.id);
        if unread_line {
            new_messages_line(ui, look);
        }
        let grouped = previous.is_some_and(|prev| {
            prev.author.id == message.author.id
                && !unread_line
                && message.created_at.saturating_sub(prev.created_at) < GROUP_MS
                && day(prev.created_at) == day(message.created_at)
                && !prev.deleted
                && (prev.reply_count == 0 || !threads)
        });
        message_item(ui, main, look, message, grouped, threads, actions);
        previous = Some(message);
    }
}

/// Keep the text column fixed even when a consecutive message has no avatar.
fn message_gutter(ui: &mut Ui, grouped: bool, draw_avatar: impl FnOnce(&mut Ui)) {
    ui.allocate_ui_with_layout(
        vec2(36.0, if grouped { 18.0 } else { 36.0 }),
        Layout::top_down(Align::Min),
        |ui| {
            if grouped {
                ui.allocate_exact_size(vec2(36.0, 18.0), egui::Sense::hover());
            } else {
                draw_avatar(ui);
            }
        },
    );
}

fn message_item(
    ui: &mut Ui,
    main: &mut Main,
    look: &Look,
    message: &Message,
    grouped: bool,
    threads: bool,
    actions: &mut Vec<Action>,
) {
    let p = &look.palette;
    let me = main.model.me.clone();
    let background = ui.painter().add(egui::Shape::Noop);
    let top = ui.cursor().min.y;
    ui.add_space(if grouped { 1.0 } else { 8.0 });
    ui.horizontal(|ui| {
        ui.add_space(16.0);
        message_gutter(ui, grouped, |ui| {
            avatar(
                ui,
                look,
                message.author.id.0,
                &message.author.display_name,
                36.0,
                None,
            );
        });
        ui.add_space(10.0);
        ui.vertical(|ui| {
            ui.set_max_width(ui.available_width() - 16.0);
            if !grouped {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(&message.author.display_name)
                            .strong()
                            .color(color(p.text_primary)),
                    );
                    ui.label(
                        RichText::new(clock(message.created_at))
                            .size(font_size::CAPTION)
                            .color(color(p.text_tertiary)),
                    );
                });
            }
            if message.deleted {
                ui.label(
                    RichText::new(tr("deleted message"))
                        .italics()
                        .color(color(p.text_tertiary)),
                );
            } else if unreadable(message) {
                ui.horizontal_wrapped(|ui| {
                    icon(ui, "lock", 14.0, color(p.text_tertiary));
                    ui.label(
                        RichText::new(tr(
                            "Encrypted before this device could read this conversation.",
                        ))
                        .italics()
                        .color(color(p.text_tertiary)),
                    );
                });
            } else {
                widgets::message_body(ui, look, &message.body, &me.username);
                if message.edited_at.is_some() {
                    ui.label(
                        RichText::new(tr("(edited)"))
                            .size(font_size::CAPTION)
                            .color(color(p.text_tertiary)),
                    );
                }
            }
            if !message.reactions.is_empty() {
                reactions(ui, look, message, me.id, actions);
            }
            if threads && message.reply_count > 0 {
                let text = if message.reply_count == 1 {
                    tr("1 reply").to_string()
                } else {
                    trf("{} replies", &[&message.reply_count.to_string()])
                };
                if ui
                    .link(RichText::new(text).strong().color(color(p.accent_text)))
                    .clicked()
                {
                    actions.push(Action::OpenThread(message.id));
                }
            }
        });
    });
    ui.add_space(2.0);
    let rect = Rect::from_min_max(
        pos2(ui.max_rect().left(), top),
        pos2(ui.max_rect().right(), ui.cursor().min.y),
    );
    let popup_open = egui::Popup::is_any_open(ui.ctx());
    if ui.rect_contains_pointer(rect) && !popup_open {
        main.hovered = Some((message.id, threads));
    }
    if main.hovered == Some((message.id, threads)) {
        if !ui.rect_contains_pointer(rect) && !popup_open {
            main.hovered = None;
        } else {
            ui.painter().set(
                background,
                egui::Shape::rect_filled(
                    rect,
                    CornerRadius::ZERO,
                    color(p.control_bg_hover).gamma_multiply(0.5),
                ),
            );
            if !message.deleted {
                toolbar(ui, main, look, message, rect, threads, actions);
            }
        }
    }
}

/// An encrypted message this device holds no key for: sent before it joined the conversation.
fn unreadable(message: &Message) -> bool {
    message.encrypted && !message.deleted && message.body.is_empty()
}

fn toolbar(
    ui: &mut Ui,
    main: &Main,
    look: &Look,
    message: &Message,
    rect: Rect,
    threads: bool,
    actions: &mut Vec<Action>,
) {
    let p = &look.palette;
    // Text this device cannot read cannot be edited from it either.
    let mine = message.author.id == main.model.me.id && !unreadable(message);
    let count =
        1 + usize::from(threads) + usize::from(mine) + usize::from(main.can_delete(message));
    let width = count as f32 * 30.0 + 8.0;
    let bar = Rect::from_min_size(
        pos2(rect.right() - width - 16.0, rect.top() - 10.0),
        vec2(width, 34.0),
    );
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(bar)
            .layout(Layout::left_to_right(Align::Center)),
    );
    egui::Frame::NONE
        .fill(color(p.surface_raised))
        .stroke(Stroke::new(1.0, color(p.border_default)))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(3))
        .show(&mut child, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let react = icon_button(ui, look, "heart", tr("Add a reaction"), false);
            egui::Popup::menu(&react).show(|ui| {
                ui.horizontal(|ui| {
                    for emoji in QUICK_REACTIONS {
                        if ui.button(RichText::new(emoji).size(18.0)).clicked() {
                            actions.push(Action::React(message.id, emoji.to_string()));
                            ui.close();
                        }
                    }
                });
            });
            if threads && icon_button(ui, look, "reply", tr("Reply in thread"), false).clicked() {
                actions.push(Action::OpenThread(message.id));
            }
            if mine && icon_button(ui, look, "pencil", tr("Edit"), false).clicked() {
                actions.push(Action::Dialog(Dialog::Edit {
                    id: message.id,
                    body: message.body.clone(),
                }));
            }
            if main.can_delete(message)
                && icon_button(ui, look, "delete", tr("Delete"), false).clicked()
            {
                actions.push(Action::Dialog(Dialog::ConfirmDelete { id: message.id }));
            }
        });
}

fn reactions(ui: &mut Ui, look: &Look, message: &Message, me: UserId, actions: &mut Vec<Action>) {
    let p = &look.palette;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        for Reaction {
            emoji,
            count,
            users,
        } in &message.reactions
        {
            let mine = users.contains(&me);
            let (fill, stroke, ink) = if mine {
                (p.accent_subtle, p.accent_default, p.accent_text)
            } else {
                (p.surface_sunken, p.border_subtle, p.text_secondary)
            };
            let button =
                egui::Button::new(RichText::new(format!("{emoji} {count}")).color(color(ink)))
                    .fill(color(fill))
                    .stroke(Stroke::new(1.0, color(stroke)))
                    .corner_radius(CornerRadius::same(10))
                    .selected(mine);
            if ui.add(button).clicked() {
                actions.push(Action::React(message.id, emoji.clone()));
            }
        }
    });
}

fn pending_list(
    ui: &mut Ui,
    look: &Look,
    model: &Model,
    pending: &[Pending],
    actions: &mut Vec<Action>,
) {
    let p = &look.palette;
    for item in pending {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            avatar(ui, look, model.me.id.0, &model.me.display_name, 36.0, None);
            ui.add_space(10.0);
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(&model.me.display_name)
                        .strong()
                        .color(color(p.text_secondary)),
                );
                ui.label(RichText::new(&item.body).color(color(p.text_secondary)));
                if item.failed {
                    ui.horizontal(|ui| {
                        if ui
                            .link(
                                RichText::new(tr("Failed to send. Click to retry."))
                                    .color(color(p.status_danger_fg)),
                            )
                            .clicked()
                        {
                            actions.push(Action::Retry(item.nonce.clone()));
                        }
                        if ui.small_button(tr("Delete")).clicked() {
                            actions.push(Action::Discard(item.nonce.clone()));
                        }
                    });
                } else {
                    ui.label(
                        RichText::new(tr("Sending…"))
                            .size(font_size::CAPTION)
                            .color(color(p.text_tertiary)),
                    );
                }
            });
        });
    }
}

fn day_separator(ui: &mut Ui, look: &Look, at: Timestamp) {
    let p = &look.palette;
    ui.add_space(8.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
    let text = date(at);
    let galley = ui.painter().layout_no_wrap(
        text,
        egui::FontId::proportional(font_size::CAPTION),
        color(p.text_secondary),
    );
    let half = galley.size().x / 2.0 + 10.0;
    let y = rect.center().y;
    let stroke = Stroke::new(1.0, color(p.border_subtle));
    ui.painter().line_segment(
        [pos2(rect.left() + 16.0, y), pos2(rect.center().x - half, y)],
        stroke,
    );
    ui.painter().line_segment(
        [
            pos2(rect.center().x + half, y),
            pos2(rect.right() - 16.0, y),
        ],
        stroke,
    );
    ui.painter().galley(
        rect.center() - galley.size() / 2.0,
        galley,
        color(p.text_secondary),
    );
}

fn new_messages_line(ui: &mut Ui, look: &Look) {
    let ink = color(look.palette.status_danger_fg);
    ui.add_space(6.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
    let y = rect.center().y;
    ui.painter().line_segment(
        [pos2(rect.left() + 16.0, y), pos2(rect.right() - 120.0, y)],
        Stroke::new(1.0, ink),
    );
    ui.painter().text(
        pos2(rect.right() - 16.0, y),
        egui::Align2::RIGHT_CENTER,
        tr("New messages"),
        egui::FontId::proportional(font_size::CAPTION),
        ink,
    );
}

fn typing_line(
    ui: &mut Ui,
    look: &Look,
    model: &Model,
    channel: ChannelId,
    thread: Option<MessageId>,
    now: f64,
) {
    let names: Vec<&str> = model
        .typists(channel, thread, now)
        .iter()
        .map(|u| u.display_name.as_str())
        .collect();
    let text = match names.as_slice() {
        [] => String::new(),
        [one] => trf("{} is typing…", &[one]),
        many => trf("{} people are typing…", &[&many.len().to_string()]),
    };
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 18.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.label(
                RichText::new(text)
                    .size(font_size::CAPTION)
                    .color(color(look.palette.text_secondary)),
            );
        },
    );
}

fn composer(
    ui: &mut Ui,
    main: &mut Main,
    look: &Look,
    channel: ChannelId,
    thread: Option<MessageId>,
    hint: &str,
    actions: &mut Vec<Action>,
) {
    let id = Id::unique(("composer", channel.0, thread.map(|t| t.0)));
    let focused = ui.memory(|m| m.has_focus(id));
    let send_key = focused && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
    let draft = main.drafts.entry((channel, thread)).or_default();
    let mut send = send_key;
    cr::field_frame(look)
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let width = ui.available_width() - 36.0;
                let response = ui.add(
                    TextEdit::multiline(draft)
                        .id(id)
                        .frame(egui::Frame::NONE)
                        .hint_text(hint)
                        .desired_rows(1)
                        .desired_width(width)
                        .lock_focus(true)
                        .return_key(Some(egui::KeyboardShortcut::new(
                            Modifiers::SHIFT,
                            Key::Enter,
                        ))),
                );
                if response.changed() && !draft.trim().is_empty() {
                    actions.push(Action::Typing { channel, thread });
                }
                let wants_focus = if thread.is_some() {
                    false
                } else {
                    main.focus_composer
                };
                if wants_focus && main.dialog.is_none() {
                    response.request_focus();
                    main.focus_composer = false;
                }
                ui.with_layout(Layout::right_to_left(Align::Max), |ui| {
                    if icon_button(ui, look, "send", tr("Send"), !draft.trim().is_empty()).clicked()
                    {
                        send = true;
                    }
                });
            });
        });
    if send {
        actions.push(Action::Send { channel, thread });
    }
}

// --- Right panel -------------------------------------------------------------------------------

fn context_panel(ui: &mut Ui, main: &mut Main, look: &Look, now: f64, actions: &mut Vec<Action>) {
    match main.panel {
        Panel::Thread if main.model.thread.is_some() => thread_panel(ui, main, look, now, actions),
        Panel::Search if main.results.is_some() => search_panel(ui, main, look, actions),
        _ => details_panel(ui, main, look, actions),
    }
}

fn panel_header(ui: &mut Ui, look: &Look, title: &str, closable: bool, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.set_height(44.0);
        ui.add_space(14.0);
        ui.label(
            RichText::new(title)
                .size(font_size::LARGE)
                .strong()
                .color(color(look.palette.text_primary)),
        );
        if closable {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(8.0);
                let (rect, _) =
                    ui.allocate_exact_size(vec2(layout::CLOSE, layout::CLOSE), Sense::hover());
                if cr::close_button(ui, look, rect, Id::unique(("close-panel", title))) {
                    actions.push(Action::ClosePanel);
                }
            });
        }
    });
    let y = ui.min_rect().bottom();
    ui.painter().line_segment(
        [
            pos2(ui.max_rect().left(), y),
            pos2(ui.max_rect().right(), y),
        ],
        Stroke::new(1.0, color(look.palette.border_subtle)),
    );
}

fn thread_panel(ui: &mut Ui, main: &mut Main, look: &Look, now: f64, actions: &mut Vec<Action>) {
    panel_header(ui, look, tr("Thread"), true, actions);
    let Some(thread) = main.model.thread.clone() else {
        return;
    };
    let channel = thread.root.channel_id;
    let pending: Vec<Pending> = main
        .model
        .pending
        .iter()
        .filter(|x| x.thread_root == Some(thread.root.id))
        .cloned()
        .collect();
    ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
        ui.add_space(10.0);
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(ui.max_rect().shrink2(vec2(12.0, 0.0)))
                .layout(Layout::bottom_up(Align::Min)),
            |ui| {
                if !thread.root.deleted {
                    composer(
                        ui,
                        main,
                        look,
                        channel,
                        Some(thread.root.id),
                        tr("Reply…"),
                        actions,
                    );
                }
                typing_line(ui, look, &main.model, channel, Some(thread.root.id), now);
            },
        );
        ui.with_layout(Layout::top_down(Align::Min), |ui| {
            egui::ScrollArea::vertical()
                .id_salt(("thread", thread.root.id.0))
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    message_item(ui, main, look, &thread.root, false, false, actions);
                    let count = thread.root.reply_count;
                    let label = if count == 0 {
                        tr("No replies yet").to_string()
                    } else if count == 1 {
                        tr("1 reply").to_string()
                    } else {
                        trf("{} replies", &[&count.to_string()])
                    };
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.add_space(16.0);
                        ui.label(
                            RichText::new(label)
                                .size(font_size::SMALL)
                                .color(color(look.palette.text_secondary)),
                        );
                    });
                    if !thread.loaded {
                        ui.spinner();
                    }
                    message_list(ui, main, look, &thread.replies, None, false, actions);
                    pending_list(ui, look, &main.model, &pending, actions);
                });
        });
    });
}

fn search_panel(ui: &mut Ui, main: &Main, look: &Look, actions: &mut Vec<Action>) {
    let p = &look.palette;
    panel_header(ui, look, tr("Search results"), true, actions);
    let Some((query, results)) = &main.results else {
        return;
    };
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                ui.label(RichText::new(format!("“{query}”")).color(color(p.text_secondary)));
            });
            if results.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    ui.label(RichText::new(tr("No results.")).color(color(p.text_tertiary)));
                });
            }
            for m in results {
                let where_ = main
                    .model
                    .channel_info(m.channel_id)
                    .map(|c| {
                        if c.kind == ChannelKind::Direct {
                            peer_name(c)
                        } else {
                            format!("#{}", c.name)
                        }
                    })
                    .unwrap_or_default();
                let (rect, response) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 62.0), Sense::click());
                if response.hovered() {
                    ui.painter().rect_filled(
                        rect.shrink2(vec2(6.0, 1.0)),
                        CornerRadius::same(10),
                        color(p.control_bg_hover),
                    );
                }
                let mut child =
                    ui.new_child(UiBuilder::new().max_rect(rect.shrink2(vec2(14.0, 6.0))));
                child.label(
                    RichText::new(format!(
                        "{where_} · {} · {}",
                        m.author.display_name,
                        clock(m.created_at)
                    ))
                    .size(font_size::CAPTION)
                    .color(color(p.text_secondary)),
                );
                child.add(
                    egui::Label::new(
                        RichText::new(hito_core::format::preview(&m.body, 140))
                            .color(color(p.text_primary)),
                    )
                    .truncate(),
                );
                if response.clicked() {
                    actions.push(Action::OpenResult(m.clone()));
                }
            }
        });
}

fn details_panel(ui: &mut Ui, main: &Main, look: &Look, actions: &mut Vec<Action>) {
    let p = &look.palette;
    panel_header(ui, look, tr("Details"), false, actions);
    let channel = main.model.current_channel().cloned();
    let ws = main.model.current_workspace().cloned();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(8.0);
            egui::Frame::NONE
                .inner_margin(Margin::symmetric(14, 0))
                .show(ui, |ui| {
                    if let Some(c) = &channel {
                        if c.kind != ChannelKind::Direct {
                            let caption = |ui: &mut Ui, t: &str| {
                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new(t)
                                        .size(font_size::CAPTION)
                                        .strong()
                                        .color(color(p.text_secondary)),
                                );
                            };
                            caption(ui, tr("Channel"));
                            ui.label(
                                RichText::new(format!("#{}", c.name))
                                    .strong()
                                    .color(color(p.text_primary)),
                            );
                            caption(ui, tr("Topic"));
                            let topic = if c.topic.is_empty() {
                                "—"
                            } else {
                                c.topic.as_str()
                            };
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new(topic).color(color(p.text_primary)));
                                let can_edit = ws.as_ref().is_some_and(|w| {
                                    hito_core::roles::can_manage_channel(
                                        w.role,
                                        c.kind,
                                        c.joined,
                                        c.created_by == Some(main.model.me.id),
                                    )
                                });
                                if can_edit
                                    && icon_button(ui, look, "pencil", tr("Edit"), false).clicked()
                                {
                                    actions.push(Action::Dialog(Dialog::Topic {
                                        channel: c.id,
                                        topic: c.topic.clone(),
                                    }));
                                }
                            });
                            caption(ui, tr("Who can see it"));
                            let who = if c.kind == ChannelKind::Private {
                                tr("invited members only")
                            } else {
                                tr("everyone in the workspace")
                            };
                            ui.label(RichText::new(who).color(color(p.text_primary)));
                            caption(ui, tr("Created"));
                            ui.label(
                                RichText::new(date(c.created_at)).color(color(p.text_primary)),
                            );
                            ui.add_space(10.0);
                            ui.horizontal(|ui| {
                                if ws.as_ref().is_some_and(|w| w.role.can_invite())
                                    && cr::secondary_button(ui, look, tr("Invite people"), true)
                                        .clicked()
                                {
                                    actions.push(Action::Dialog(Dialog::Invite {
                                        invite: None,
                                        uses: 5,
                                        hours: 168,
                                    }));
                                }
                                if c.joined
                                    && cr::secondary_button(ui, look, tr("Leave channel"), true)
                                        .clicked()
                                {
                                    actions.push(Action::LeaveChannel(c.id));
                                }
                            });
                        }
                    }
                    if let Some(c) = &channel {
                        encryption(ui, main, look, c.id);
                    }
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new(tr("People"))
                            .size(font_size::CAPTION)
                            .strong()
                            .color(color(p.text_secondary)),
                    );
                    ui.add_space(4.0);
                    let mut members = ws
                        .and_then(|w| main.model.members.get(&w.id).cloned())
                        .unwrap_or_default();
                    members.sort_by_key(|m| {
                        (
                            !main.model.online.contains(&m.user.id),
                            m.user.display_name.to_lowercase(),
                        )
                    });
                    for m in members {
                        let online = main.model.online.contains(&m.user.id);
                        let (rect, response) = ui
                            .allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
                        if response.hovered() {
                            ui.painter().rect_filled(
                                rect,
                                CornerRadius::same(10),
                                color(p.control_bg_hover),
                            );
                        }
                        let mut child = ui.new_child(
                            UiBuilder::new()
                                .max_rect(rect.shrink2(vec2(6.0, 0.0)))
                                .layout(Layout::left_to_right(Align::Center)),
                        );
                        avatar(
                            &mut child,
                            look,
                            m.user.id.0,
                            &m.user.display_name,
                            24.0,
                            Some(online),
                        );
                        child.add_space(8.0);
                        child.label(
                            RichText::new(&m.user.display_name).color(color(p.text_primary)),
                        );
                        child.label(
                            RichText::new(format!("@{}", m.user.username))
                                .size(font_size::CAPTION)
                                .color(color(p.text_tertiary)),
                        );
                        if response.clicked() && m.user.id != main.model.me.id {
                            actions.push(Action::OpenDirect(m.user.id));
                        }
                    }
                });
        });
}

/// Which devices can read the open conversation, with the fingerprints people can compare.
fn encryption(ui: &mut Ui, main: &Main, look: &Look, channel: ChannelId) {
    let p = &look.palette;
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        icon(ui, "lock", 14.0, color(p.accent_text));
        ui.label(
            RichText::new(tr("Encrypted end to end"))
                .size(font_size::CAPTION)
                .strong()
                .color(color(p.text_secondary)),
        );
    });
    ui.label(
        RichText::new(tr(
            "Only these devices can read what is said here. The server cannot. Compare fingerprints in person to be sure.",
        ))
        .size(font_size::CAPTION)
        .color(color(p.text_tertiary)),
    );
    ui.add_space(4.0);
    let fingerprint = |ui: &mut Ui, name: &str, print: &str, this: bool| {
        ui.horizontal_wrapped(|ui| {
            let label = if this {
                trf("{} (this device)", &[name])
            } else {
                name.to_string()
            };
            ui.label(RichText::new(label).color(color(p.text_primary)));
            ui.label(
                RichText::new(print)
                    .monospace()
                    .size(font_size::CAPTION)
                    .color(color(p.text_tertiary)),
            );
        });
    };
    let mine = main.secure.fingerprint();
    let devices = main.model.devices.get(&channel);
    let names = main
        .model
        .workspace
        .and_then(|ws| main.model.members.get(&ws));
    match devices {
        Some(list) if !list.is_empty() => {
            for leaf in list {
                let print = hito_e2e::fingerprint(&leaf.public_key);
                let name = names
                    .and_then(|l| l.iter().find(|m| m.user.id == leaf.user))
                    .map_or_else(
                        || tr("Someone").to_string(),
                        |m| m.user.display_name.clone(),
                    );
                fingerprint(ui, &name, &print, print == mine);
            }
        }
        _ => fingerprint(ui, &main.model.me.display_name, mine, true),
    }
}

// --- Toasts and time -------------------------------------------------------------------------------

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

fn local(at: Timestamp) -> Option<chrono::DateTime<chrono::Local>> {
    use chrono::TimeZone as _;
    chrono::Local.timestamp_millis_opt(at).single()
}

fn day(at: Timestamp) -> Option<chrono::NaiveDate> {
    local(at).map(|t| t.date_naive())
}

pub fn clock(at: Timestamp) -> String {
    local(at)
        .map(|t| t.format("%H:%M").to_string())
        .unwrap_or_default()
}

pub fn date(at: Timestamp) -> String {
    local(at)
        .map(|t| t.format("%d/%m/%Y").to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grouped_messages_keep_the_same_text_column() {
        let ctx = egui::Context::default();
        let mut positions = Vec::new();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            for grouped in [false, true] {
                ui.horizontal(|ui| {
                    message_gutter(ui, grouped, |ui| {
                        ui.allocate_exact_size(vec2(36.0, 36.0), egui::Sense::hover());
                    });
                    ui.add_space(10.0);
                    positions.push(ui.cursor().min.x);
                });
            }
        });
        output.textures_delta.clear();
        assert_eq!(positions.len(), 2);
        assert_eq!(positions.first(), positions.last());
    }
}
