//! The window frame when Hito draws its own (Linux, unless the person picked the system's):
//! the empty part of the top bar moves the window, a double click maximizes it, the edges
//! resize it, and the NDL controls sit on the right. Windows keeps its native frame. macOS keeps
//! its native traffic lights, but inside the window: the content runs under a transparent title
//! bar, the top bar leaves room for the lights on its left, and its empty part moves the window.

use egui::{pos2, CursorIcon, PointerButton, Rect, ResizeDirection, Sense, Ui, ViewportCommand};
use nodalia_theme::{space, window_controls, Mode, Palette};

use crate::i18n::tr;

/// Whether this platform may draw NDL window controls at all.
pub const NDL_CONTROLS_POSSIBLE: bool = cfg!(target_os = "linux");

/// Whether the window's content runs under a transparent title bar (macOS).
pub const INSET_TITLE_BAR: bool = cfg!(target_os = "macos");

/// Room the macOS traffic lights take at the left of the top bar, with the gap to the first
/// control (nodalia-design's, the same in every Nodalia app).
pub const TRAFFIC_LIGHTS_WIDTH: f32 = nodalia_theme::titlebar::MAC_LIGHTS;

/// The native frame's options for this platform: on macOS the content fills the window and the
/// traffic lights float over the top bar, as in the other Nodalia apps.
pub fn frame(viewport: egui::ViewportBuilder, ndl_controls: bool) -> egui::ViewportBuilder {
    if INSET_TITLE_BAR {
        viewport
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false)
            .with_titlebar_buttons_shown(true)
    } else {
        viewport.with_decorations(!ndl_controls)
    }
}

/// Space to leave at the left of the top bar before its first widget.
pub fn leading_inset() -> f32 {
    if INSET_TITLE_BAR {
        TRAFFIC_LIGHTS_WIDTH
    } else {
        0.0
    }
}

/// Whether Hito moves the window from the top bar itself (it draws the frame, or the native
/// title bar is hidden behind the content).
pub fn drags_window(ndl_controls: bool) -> bool {
    ndl_controls || INSET_TITLE_BAR
}

/// Width the controls take at the right of the bar, margin included.
pub const CONTROLS_WIDTH: f32 = window_controls::WIDTH + space::SM;

const EDGE: f32 = 5.0;

/// Make the empty area of the bar drag and maximize the window. Call before drawing the bar's
/// own widgets, so they sit on top of it.
pub fn drag_area(ui: &Ui, rect: Rect) {
    let response = ui.interact(
        rect,
        ui.scope_id().with("hito_title_drag"),
        Sense::click_and_drag(),
    );
    if response.drag_started_by(PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
    if response.double_clicked() {
        let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx()
            .send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    }
}

/// Draw the NDL window controls at the right of `row` and act on them.
pub fn controls(ui: &Ui, row: Rect, palette: &Palette, mode: Mode) {
    let (focused, maximized) = ui.input(|i| (i.focused, i.viewport().maximized.unwrap_or(false)));
    let right = row.right() - space::SM;
    let rect = Rect::from_min_max(
        pos2(right - window_controls::WIDTH, row.top()),
        pos2(right, row.bottom()),
    );
    let labels = window_controls::Labels {
        minimize: tr("Minimize"),
        maximize: tr("Maximize"),
        restore: tr("Restore"),
        close: tr("Close"),
    };
    if let Some(action) = window_controls::show(ui, rect, palette, mode, focused, maximized, labels)
    {
        ui.ctx().send_viewport_cmd(command(action));
    }
}

fn command(action: window_controls::Action) -> ViewportCommand {
    use window_controls::Action;
    match action {
        Action::Minimize => ViewportCommand::Minimized(true),
        Action::Maximize => ViewportCommand::Maximized(true),
        Action::Restore => ViewportCommand::Maximized(false),
        Action::Close => ViewportCommand::Close,
    }
}

/// Resize handles along the window's edges, when it is not maximized.
pub fn resize_edges(ui: &Ui, full: Rect) {
    if ui.input(|i| {
        i.viewport().maximized.unwrap_or(false) || i.viewport().fullscreen.unwrap_or(false)
    }) {
        return;
    }
    for (index, (direction, cursor, rect)) in bands(full).into_iter().enumerate() {
        let response = ui
            .interact(
                rect,
                ui.scope_id().with(("hito_resize", index)),
                Sense::drag(),
            )
            .on_hover_cursor(cursor);
        if response.drag_started_by(PointerButton::Primary) {
            ui.ctx()
                .send_viewport_cmd(ViewportCommand::BeginResize(direction));
        }
    }
}

fn bands(full: Rect) -> [(ResizeDirection, CursorIcon, Rect); 8] {
    use {CursorIcon as C, ResizeDirection as D};
    let (l, t, r, b) = (full.left(), full.top(), full.right(), full.bottom());
    let e = EDGE;
    let k = 3.0 * EDGE;
    let rect = |x0, y0, x1, y1| Rect::from_min_max(pos2(x0, y0), pos2(x1, y1));
    [
        (D::North, C::ResizeNorth, rect(l + k, t, r - k, t + e)),
        (D::South, C::ResizeSouth, rect(l + k, b - e, r - k, b)),
        (D::West, C::ResizeWest, rect(l, t + k, l + e, b - k)),
        (D::East, C::ResizeEast, rect(r - e, t + k, r, b - k)),
        (D::NorthWest, C::ResizeNorthWest, rect(l, t, l + k, t + e)),
        (D::NorthEast, C::ResizeNorthEast, rect(r - k, t, r, t + e)),
        (D::SouthWest, C::ResizeSouthWest, rect(l, b - e, l + k, b)),
        (D::SouthEast, C::ResizeSouthEast, rect(r - k, b - e, r, b)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_frame_fits_the_platform() {
        let viewport = frame(egui::ViewportBuilder::default(), false);
        if cfg!(target_os = "macos") {
            assert_eq!(viewport.fullsize_content_view, Some(true));
            assert_eq!(viewport.titlebar_shown, Some(false));
            assert_ne!(
                viewport.titlebar_buttons_shown,
                Some(false),
                "keep the traffic lights"
            );
            assert!(leading_inset() > 0.0 && drags_window(false));
        } else {
            assert_eq!(viewport.fullsize_content_view, None, "a native frame");
            assert_eq!(viewport.decorations, Some(true));
            assert_eq!(leading_inset(), 0.0);
            assert!(!drags_window(false));
        }
        let own = frame(egui::ViewportBuilder::default(), true);
        if !cfg!(target_os = "macos") {
            assert_eq!(own.decorations, Some(false));
        }
    }

    #[test]
    fn controls_map_to_viewport_commands() {
        use window_controls::Action;
        assert_eq!(command(Action::Minimize), ViewportCommand::Minimized(true));
        assert_eq!(command(Action::Maximize), ViewportCommand::Maximized(true));
        assert_eq!(command(Action::Restore), ViewportCommand::Maximized(false));
        assert_eq!(command(Action::Close), ViewportCommand::Close);
    }

    #[test]
    fn resize_bands_stay_on_the_edges() {
        let full = Rect::from_min_max(pos2(0.0, 0.0), pos2(800.0, 600.0));
        let inner = full.shrink(EDGE);
        for (_, _, band) in bands(full) {
            assert!(band.is_positive());
            assert!(full.contains_rect(band));
            assert!(!band.intersect(inner).is_positive());
        }
    }
}
