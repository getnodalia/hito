//! Small pieces drawn with the NDL palette: icons, icon buttons, avatars, badges, the message
//! text renderer.

use hito_core::format::{self, Block, Inline};
use egui::{
    vec2, Color32, CornerRadius, FontId, Frame, Margin, Response, RichText, Sense, Stroke, Ui,
};
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

/// Render a message body: paragraphs, quotes and code blocks, with bold, italic, strike,
/// code spans, links and mentions. Nothing in the body is ever interpreted as markup beyond
/// that, and links only open on a click.
pub fn message_body(ui: &mut Ui, look: &Look, body: &str, me: &str) {
    let p = &look.palette;
    for block in format::parse(body) {
        match block {
            Block::Paragraph(spans) => inline_row(ui, look, &spans, me),
            Block::Quote(spans) => {
                Frame::NONE
                    .inner_margin(Margin {
                        left: 10,
                        right: 0,
                        top: 2,
                        bottom: 2,
                    })
                    .show(ui, |ui| {
                        let top = ui.cursor().min;
                        inline_row(ui, look, &spans, me);
                        let bottom = ui.min_rect().bottom();
                        ui.painter().line_segment(
                            [
                                egui::pos2(top.x - 8.0, top.y),
                                egui::pos2(top.x - 8.0, bottom),
                            ],
                            Stroke::new(3.0, color(p.border_strong)),
                        );
                    });
            }
            Block::Code { code, .. } => {
                Frame::NONE
                    .fill(color(p.surface_sunken))
                    .stroke(Stroke::new(1.0, color(p.border_subtle)))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::same(8))
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.label(RichText::new(code).monospace().color(color(p.text_primary)));
                    });
            }
        }
    }
}

fn inline_row(ui: &mut Ui, look: &Look, spans: &[Inline], me: &str) {
    if !spans.iter().any(|s| matches!(s, Inline::Link(_))) {
        // One galley for the paragraph, so it wraps like text rather than like a row of labels.
        ui.label(paragraph(ui, look, spans, me));
        return;
    }
    let p = &look.palette;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for span in spans {
            match span {
                Inline::Text(text) => {
                    let mut lines = text.split('\n').peekable();
                    while let Some(line) = lines.next() {
                        if !line.is_empty() {
                            ui.label(RichText::new(line).color(color(p.text_primary)));
                        }
                        if lines.peek().is_some() {
                            ui.end_row();
                        }
                    }
                }
                Inline::Bold(t) => {
                    ui.label(RichText::new(t).strong().color(color(p.text_primary)));
                }
                Inline::Italic(t) => {
                    ui.label(RichText::new(t).italics().color(color(p.text_primary)));
                }
                Inline::Strike(t) => {
                    ui.label(
                        RichText::new(t)
                            .strikethrough()
                            .color(color(p.text_secondary)),
                    );
                }
                Inline::Code(t) => {
                    ui.label(
                        RichText::new(t)
                            .monospace()
                            .background_color(color(p.surface_sunken))
                            .color(color(p.text_primary)),
                    );
                }
                Inline::Link(url) => {
                    ui.hyperlink_to(RichText::new(url).color(color(p.text_link)), url);
                }
                Inline::Mention(name) => {
                    let ink = if name == me {
                        color(p.text_on_accent)
                    } else {
                        color(p.accent_text)
                    };
                    let fill = if name == me {
                        color(p.accent_default)
                    } else {
                        color(p.accent_subtle)
                    };
                    ui.label(
                        RichText::new(format!("@{name}"))
                            .strong()
                            .color(ink)
                            .background_color(fill),
                    );
                }
            }
        }
    });
}

/// A paragraph without links as one layout job.
fn paragraph(ui: &Ui, look: &Look, spans: &[Inline], me: &str) -> egui::text::LayoutJob {
    use egui::text::{LayoutJob, TextFormat};
    let p = &look.palette;
    let body = egui::TextStyle::Body.resolve(ui.style());
    let mono = egui::TextStyle::Monospace.resolve(ui.style());
    let plain = TextFormat::simple(body.clone(), color(p.text_primary));
    let mut job = LayoutJob::default();
    for span in spans {
        match span {
            Inline::Text(t) => job.append(t, 0.0, plain.clone()),
            Inline::Bold(t) => {
                job.append(
                    t,
                    0.0,
                    TextFormat {
                        color: ui.visuals().strong_text_color(),
                        ..plain.clone()
                    },
                );
            }
            Inline::Italic(t) => job.append(
                t,
                0.0,
                TextFormat {
                    italics: true,
                    ..plain.clone()
                },
            ),
            Inline::Strike(t) => job.append(
                t,
                0.0,
                TextFormat {
                    color: color(p.text_secondary),
                    strikethrough: Stroke::new(1.0, color(p.text_secondary)),
                    ..plain.clone()
                },
            ),
            Inline::Code(t) => job.append(
                t,
                0.0,
                TextFormat {
                    font_id: mono.clone(),
                    background: color(p.surface_sunken),
                    ..plain.clone()
                },
            ),
            Inline::Mention(name) => {
                let mine = name == me;
                let ink = if mine {
                    color(p.text_on_accent)
                } else {
                    color(p.accent_text)
                };
                let fill = if mine {
                    color(p.accent_default)
                } else {
                    color(p.accent_subtle)
                };
                job.append(
                    &format!("@{name}"),
                    0.0,
                    TextFormat {
                        color: ink,
                        background: fill,
                        ..plain.clone()
                    },
                );
            }
            Inline::Link(url) => job.append(
                url,
                0.0,
                TextFormat {
                    color: color(p.text_link),
                    ..plain.clone()
                },
            ),
        }
    }
    job.wrap.max_width = ui.available_width();
    job
}
