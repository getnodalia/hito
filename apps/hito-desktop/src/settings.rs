//! What the app remembers between launches: preferences in `settings.json`, and the session
//! token, only when the person asked to stay signed in, in `session.json` (owner-only on Unix).
//!
//! Both live in the platform's config folder (`~/.config/hito` on Linux). `HITO_DESKTOP_DIR`
//! points the app elsewhere, which is how two clients run side by side on one computer.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::i18n::Lang;

/// Light, dark or the system's choice, each with an optional high-contrast palette.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

/// Which window buttons the Linux build draws. macOS and Windows always use their own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowControls {
    #[default]
    Nodalia,
    System,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The last server address typed on the sign-in screen.
    pub server: String,
    pub username: String,
    pub theme: Theme,
    pub high_contrast: bool,
    pub reduced_motion: bool,
    pub window_controls: WindowControls,
    /// `None` follows the system language.
    pub language: Option<Lang>,
    /// Keep the session after the app closes.
    pub remember: bool,
    /// Show the context panel on the right.
    pub context_panel: bool,
    /// Generic OS notifications on supported systems; no message text.
    pub notifications: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            server: "localhost:7879".into(),
            username: String::new(),
            theme: Theme::System,
            high_contrast: false,
            reduced_motion: false,
            window_controls: WindowControls::Nodalia,
            language: None,
            remember: true,
            context_panel: true,
            notifications: false,
        }
    }
}

/// A saved session: the server it belongs to and its bearer token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSession {
    pub server: String,
    pub token: String,
}

/// Where the files live. `None` when the platform has no config folder; nothing is saved then.
pub fn dir() -> Option<PathBuf> {
    if let Some(custom) = std::env::var_os("HITO_DESKTOP_DIR") {
        return Some(PathBuf::from(custom));
    }
    directories::ProjectDirs::from("dev", "danielmiguelt", "Hito")
        .map(|d| d.config_dir().to_path_buf())
}

impl Settings {
    pub fn load(dir: Option<&Path>) -> Settings {
        dir.and_then(|d| std::fs::read(d.join("settings.json")).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, dir: Option<&Path>) -> std::io::Result<()> {
        let Some(dir) = dir else { return Ok(()) };
        let json = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        write_private(&dir.join("settings.json"), &json)
    }
}

pub fn load_session(dir: Option<&Path>) -> Option<SavedSession> {
    let bytes = std::fs::read(dir?.join("session.json")).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn save_session(dir: Option<&Path>, session: &SavedSession) -> std::io::Result<()> {
    let Some(dir) = dir else { return Ok(()) };
    let json = serde_json::to_vec(session).map_err(std::io::Error::other)?;
    write_private(&dir.join("session.json"), &json)
}

pub fn forget_session(dir: Option<&Path>) {
    if let Some(dir) = dir {
        let _ = std::fs::remove_file(dir.join("session.json"));
    }
}

/// Where this computer keeps its encryption device for `user` on `server`. The file holds the
/// device's private keys and the text it has decrypted, so it is written like the session file.
pub fn device_path(dir: Option<&Path>, server: &str, user: hito_core::UserId) -> Option<PathBuf> {
    let host: String = server
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    Some(dir?.join(format!("device-{host}-{}.json", user.0)))
}

/// Write through a temporary file and rename, so a crash never leaves half a file, with the
/// permissions set before any byte is written.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hito-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn settings_round_trip_and_tolerate_missing_fields() {
        let dir = temp_dir("round");
        let s = Settings {
            server: "chat.example.org".into(),
            theme: Theme::Dark,
            language: Some(Lang::En),
            ..Settings::default()
        };
        s.save(Some(&dir)).unwrap();
        assert_eq!(Settings::load(Some(&dir)), s);

        std::fs::write(dir.join("settings.json"), br#"{"username":"ana"}"#).unwrap();
        let partial = Settings::load(Some(&dir));
        assert_eq!(partial.username, "ana");
        assert_eq!(partial.server, Settings::default().server);

        std::fs::write(dir.join("settings.json"), b"not json").unwrap();
        assert_eq!(Settings::load(Some(&dir)), Settings::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_session_file_is_private_and_can_be_forgotten() {
        let dir = temp_dir("session");
        let saved = SavedSession {
            server: "http://localhost:7879/".into(),
            token: "secret".into(),
        };
        save_session(Some(&dir), &saved).unwrap();
        assert_eq!(load_session(Some(&dir)), Some(saved));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(dir.join("session.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        forget_session(Some(&dir));
        assert_eq!(load_session(Some(&dir)), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
