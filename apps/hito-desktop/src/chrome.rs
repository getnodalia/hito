//! The window frame when Hito draws its own (Linux, unless the person picked the system's) and
//! the inset on macOS. It is the Enterprise suite's (`nodalia_theme::enterprise::chrome`, shared
//! with Ágora); this file only gives the window controls their Spanish and English labels.

use egui::{Rect, Ui};
use nodalia_theme::{window_controls, Mode, Palette};

pub use nodalia_theme::enterprise::chrome::{
    drag_area, drags_window, frame, resize_edges, NDL_CONTROLS_POSSIBLE,
};

use crate::i18n::tr;

/// Draw the NDL window controls at the desktop's side of `row` and act on them.
pub fn controls(ui: &Ui, row: Rect, palette: &Palette, mode: Mode) {
    let labels = window_controls::Labels {
        minimize: tr("Minimize"),
        maximize: tr("Maximize"),
        restore: tr("Restore"),
        close: tr("Close"),
    };
    nodalia_theme::enterprise::chrome::controls(ui, row, palette, mode, labels);
}
