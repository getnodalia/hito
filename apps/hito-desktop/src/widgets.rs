//! Small pieces drawn with the NDL palette: icons, icon buttons, avatars, badges, progress bars
//! and the status and priority chips.

use egui::{
    vec2, Color32, CornerRadius, FontId, Frame, Margin, Response, RichText, Sense, Stroke, Ui,
};
use hito_core::{Priority, Progress, Status};
use nodalia_theme::creator::Look;
use nodalia_theme::for_egui::color;
use nodalia_theme::{font_size, icons};

/// Paint an NDL icon in a square of `size` points.
pub fn icon(ui: &mut Ui, name: &str, size: f32, ink: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    if let Some(i) = icons::get(name) {
        icons::paint(ui.painter(), rect, i, icons::Style::bold(ink));
    }
    response
}

/// A round, borderless icon button with an accessible label.
pub fn icon_button(ui: &mut Ui, look: &Look, name: &str, label: &str, active: bool) -> Response {
    let size = 28.0;
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let p = &look.palette;
    let fill = if active {
        color(p.accent_subtle)
    } else if response.hovered() {
        color(p.control_bg_hover)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(10), fill);
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(2.0),
            CornerRadius::same(12),
            Stroke::new(2.0, color(p.border_focus)),
            egui::StrokeKind::Outside,
        );
    }
    let ink = if active {
        color(p.accent_text)
    } else {
        color(p.text_secondary)
    };
    if let Some(i) = icons::get(name) {
        icons::paint(ui.painter(), rect.shrink(6.0), i, icons::Style::bold(ink));
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    response.on_hover_text(label)
}

/// Colours for avatars, from the palette's status and accent colours so they follow the theme.
fn avatar_fill(look: &Look, seed: i64) -> Color32 {
    let p = &look.palette;
    let choices = [
        p.accent_default,
        p.status_info_fg,
        p.status_success_fg,
        p.status_warning_fg,
        p.status_danger_fg,
    ];
    let i = usize::try_from(seed.rem_euclid(choices.len() as i64)).unwrap_or(0);
    choices.get(i).map_or(Color32::GRAY, |c| color(*c))
}

/// A round avatar with the person's initial.
pub fn avatar(ui: &mut Ui, look: &Look, seed: i64, name: &str, size: f32, online: Option<bool>) {
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let painter = ui.painter();
    painter.circle_filled(rect.center(), size / 2.0, avatar_fill(look, seed));
    let initial: String = name
        .chars()
        .next()
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_default();
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        initial,
        FontId::proportional(size * 0.45),
        color(look.palette.text_on_accent),
    );
    if let Some(online) = online {
        let r = (size * 0.16).max(3.5);
        let c = rect.right_bottom() - vec2(r, r);
        painter.circle_filled(c, r + 1.5, color(look.palette.surface_panel));
        if online {
            painter.circle_filled(c, r, color(look.palette.status_success_fg));
        } else {
            painter.circle_stroke(
                c,
                r - 0.5,
                Stroke::new(1.5, color(look.palette.text_tertiary)),
            );
        }
    }
}

/// The top bar's search field: NDL's field frame with a search glyph, the text centred on the
/// control's height. (`creator::search_field` lets egui's tall interact size push the text to the
/// top; this stays here until nodalia-design centres it.)
pub fn search_field(
    ui: &mut Ui,
    look: &Look,
    text: &mut String,
    hint: &str,
    width: f32,
) -> Response {
    let p = &look.palette;
    nodalia_theme::creator::field_frame(look)
        .inner_margin(Margin::symmetric(8, 0))
        .show(ui, |ui| {
            ui.set_width(width - 18.0);
            ui.horizontal(|ui| {
                let (glyph, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                if let Some(i) = icons::get("search") {
                    icons::paint(
                        ui.painter(),
                        glyph,
                        i,
                        icons::Style::regular(color(p.text_secondary)),
                    );
                }
                ui.add(
                    egui::TextEdit::singleline(text)
                        .frame(Frame::NONE)
                        .align(egui::Align2::LEFT_CENTER)
                        .hint_text(hint)
                        .desired_width(ui.available_width()),
                )
            })
            .inner
        })
        .inner
}

/// A pill with a count, for unread messages (`strong`) or mentions.
pub fn badge(ui: &mut Ui, look: &Look, count: u32, strong: bool) {
    let text = if count > 99 {
        "99+".to_string()
    } else {
        count.to_string()
    };
    let p = &look.palette;
    let (fill, ink) = if strong {
        (color(p.accent_default), color(p.text_on_accent))
    } else {
        (color(p.control_bg_hover), color(p.text_secondary))
    };
    Frame::NONE
        .fill(fill)
        .corner_radius(CornerRadius::same(9))
        .inner_margin(Margin::symmetric(6, 1))
        .show(ui, |ui| {
            ui.label(
                RichText::new(text)
                    .size(font_size::CAPTION)
                    .color(ink)
                    .strong(),
            );
        });
}
/// A thin rounded progress bar, `width` wide (the available width when 0), with the percent
/// beside it when `label` is set.
pub fn progress_bar(ui: &mut Ui, look: &Look, progress: Progress, width: f32, label: bool) {
    let p = &look.palette;
    let width = if width <= 0.0 {
        ui.available_width() - if label { 44.0 } else { 0.0 }
    } else {
        width
    };
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(width.max(24.0), 8.0), Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(4), color(p.control_bg_hover));
        let fraction = progress.fraction().clamp(0.0, 1.0);
        if fraction > 0.0 {
            let mut done = rect;
            done.set_width((rect.width() * fraction).max(8.0));
            let fill = if progress.complete() {
                color(p.status_success_fg)
            } else {
                color(p.accent_default)
            };
            painter.rect_filled(done, CornerRadius::same(4), fill);
        }
        if label {
            ui.label(
                RichText::new(format!("{} %", progress.percent()))
                    .size(font_size::SMALL)
                    .color(color(p.text_secondary)),
            );
        }
    });
}

/// The colours of a status: (text, background).
pub fn status_colors(look: &Look, status: Status) -> (Color32, Color32) {
    let p = &look.palette;
    match status {
        Status::Todo => (color(p.text_secondary), color(p.control_bg_hover)),
        Status::InProgress => (color(p.status_info_fg), color(p.status_info_bg)),
        Status::InReview => (color(p.status_warning_fg), color(p.status_warning_bg)),
        Status::Done => (color(p.status_success_fg), color(p.status_success_bg)),
    }
}

/// The ink of a priority: urgent and high stand out, normal and low stay quiet.
pub fn priority_ink(look: &Look, priority: Priority) -> Color32 {
    let p = &look.palette;
    match priority {
        Priority::Urgent => color(p.status_danger_fg),
        Priority::High => color(p.status_warning_fg),
        Priority::Normal => color(p.text_secondary),
        Priority::Low => color(p.text_tertiary),
    }
}

/// A small rounded label in a colour pair, painted at its own height whatever the row's.
pub fn chip(ui: &mut Ui, text: &str, ink: Color32, fill: Color32) -> Response {
    let galley = ui.painter().layout_no_wrap(
        text.to_string(),
        FontId::proportional(font_size::CAPTION),
        ink,
    );
    let size = galley.size() + vec2(14.0, 4.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same((size.y / 2.0) as u8), fill);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, ink);
    response
}

/// Lay out `add` from the right edge, on the current row. `with_layout` would claim the whole
/// height left in a horizontal row and push the row's height to it.
pub fn trailing<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let row = if ui.layout().is_horizontal() {
        ui.min_rect().height()
    } else {
        0.0
    };
    let height = row.max(ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), height),
        egui::Layout::right_to_left(egui::Align::Center),
        add,
    )
    .inner
}
