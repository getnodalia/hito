//! In-app updates: a quiet check when Hito starts, "Check for updates…" in the main menu, and
//! the windows that offer a new release and install it. The flow and the windows are
//! `nodalia-update`'s, shared with the other Nodalia apps; this file is what is Hito's own.

use nodalia_update::egui::{Action, Updater};

/// Where Hito's releases are, and how this build is called.
pub const APP: nodalia_update::App = nodalia_update::App {
    name: "Hito",
    repo: "getnodalia/hito",
    bundle: "Hito.app",
    version: env!("CARGO_PKG_VERSION"),
    edition: None,
};

/// Hito's accent, `#3D9A2B`, for the update windows' main button.
const ACCENT: [u8; 3] = [0x3D, 0x9A, 0x2B];

pub struct Runtime {
    updater: Updater,
    /// The check at start was asked for; `tick` starts it once it has the context.
    startup: bool,
    /// "Check for updates…" was chosen; the next `tick` starts it.
    manual: bool,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            updater: Updater::new(APP).with_accent(ACCENT),
            startup: false,
            manual: false,
        }
    }
}

impl Runtime {
    /// Ask for a check; the next frame's `tick` starts it. A manual one reports every outcome,
    /// the one at start says something only when there is a new version.
    pub fn start(&mut self, manual: bool) {
        if manual {
            self.manual = true;
        } else {
            self.startup = true;
        }
    }

    /// Starts what was asked for and collects finished work. Call once per frame.
    pub fn tick(&mut self, ctx: &egui::Context) {
        if std::mem::take(&mut self.manual) {
            self.updater.check_now(ctx);
        } else if std::mem::take(&mut self.startup) {
            self.updater.check_at_start(ctx);
        }
        let action = self.updater.poll();
        act(ctx, action);
    }

    /// Draws the update windows, if one is open.
    pub fn show(&mut self, ctx: &egui::Context) {
        let language = match crate::i18n::current() {
            crate::i18n::Lang::Es => nodalia_update::Language::Spanish,
            crate::i18n::Lang::En => nodalia_update::Language::English,
        };
        // Nothing in Hito waits to be saved: tasks are on the server the moment they change.
        let action = self.updater.show(ctx, language, true);
        act(ctx, action);
    }
}

/// Carries out what the update windows asked for.
fn act(ctx: &egui::Context, action: Action) {
    match action {
        Action::OpenUrl(url) => ctx.open_url(egui::OpenUrl::new_tab(url)),
        Action::Exit => {
            // The new version is starting, or a helper waits for this process to end. Close the
            // window, and end the process anyway if it has not ended soon.
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_secs(5));
                std::process::exit(0);
            });
        }
        Action::None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_releases_are_hitos_own() {
        assert_eq!(APP.repo, "getnodalia/hito");
        assert_eq!(APP.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(APP.bundle, "Hito.app");
    }

    #[test]
    fn the_check_at_start_is_silent_until_it_finds_something() {
        let mut rt = Runtime::default();
        assert!(!rt.updater.is_open());
        rt.updater
            .set_status(nodalia_update::egui::Status::Checking { user: false });
        assert!(!rt.updater.is_open());
        rt.updater
            .set_status(nodalia_update::egui::Status::Checking { user: true });
        assert!(rt.updater.is_open());
    }

    #[test]
    fn the_menu_asks_for_a_check_that_answers() {
        let mut rt = Runtime::default();
        rt.start(true);
        assert!(rt.manual && !rt.startup);
        rt.start(false);
        assert!(rt.startup);
    }
}
