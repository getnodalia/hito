//! Small pieces drawn with the NDL palette. The ones every Enterprise app draws the same way
//! (icons, icon buttons, avatars, badges, chips, the search field) are the suite's; what stays
//! here is Hito's: progress, status and priority.

use egui::Color32;
use hito_core::{Priority, Progress, Status};
use nodalia_theme::creator::Look;
use nodalia_theme::for_egui::color;

pub use nodalia_theme::enterprise::{avatar, chip, icon, icon_button, trailing};

/// A thin rounded progress bar, `width` wide (the available width when 0), with the percent
/// beside it when `label` is set.
pub fn progress_bar(ui: &mut egui::Ui, look: &Look, progress: Progress, width: f32, label: bool) {
    let text = format!("{} %", progress.percent());
    nodalia_theme::enterprise::progress_bar(
        ui,
        look,
        progress.fraction(),
        progress.complete(),
        width,
        label.then_some(text.as_str()),
    );
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
