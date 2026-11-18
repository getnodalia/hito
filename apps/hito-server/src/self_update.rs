//! `hito-server self-update`: install the latest stable release on this machine.
//!
//! Run as root, normally by the `hito-server-update.timer` systemd timer once a day. It:
//!
//! 1. reads the newest stable tag from the `github.com/getnodalia/hito/releases/latest` redirect
//!    (a web redirect, not the rate-limited API);
//! 2. stops there unless that version is newer than this binary;
//! 3. downloads the release's package for this installation (the `.deb`, the `.rpm`, or the
//!    `.tar.gz` that `install.sh` came from) and checks it against the release's `SHA256SUMS`;
//! 4. snapshots the database into `backups/` next to it;
//! 5. installs it: `dpkg -i` or `rpm -U` for a package (their scripts restart the service), or
//!    swaps the binary for a `.tar.gz` install, keeping the previous one to roll back to;
//! 6. checks `/healthz`, and for a binary install puts the previous version back if the new one
//!    does not answer.
//!
//! With `--auto` (the timer) it does nothing when `[updates] automatic = false`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use hito_server::Config;
use semver::Version;
use sha2::{Digest, Sha256};

const REPO: &str = "getnodalia/hito";
/// Packages are tens of MB; anything far past that is not ours.
const MAX_PACKAGE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_SUMS_BYTES: usize = 64 * 1024;
/// Backups made before updates that are kept; older ones are removed.
const KEEP_BACKUPS: usize = 3;

type Error = Box<dyn std::error::Error>;

/// How this server was installed, which decides what to download and how to install it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Install {
    Deb,
    Rpm,
    /// The binary at this path, from the `.tar.gz` and `install.sh`.
    Binary(PathBuf),
}

impl Install {
    fn asset(&self, version: &Version, arch: &str) -> String {
        let ext = match self {
            Install::Deb => "deb",
            Install::Rpm => "rpm",
            Install::Binary(_) => "tar.gz",
        };
        format!("hito-server-{version}-linux-{arch}.{ext}")
    }
}

pub async fn run(config: &Config, auto: bool) -> Result<String, Error> {
    if auto && !config.updates.automatic {
        return Ok("Automatic updates are off ([updates] automatic = false).".into());
    }
    let installed = Version::parse(env!("CARGO_PKG_VERSION"))?;
    let client = reqwest::Client::builder()
        .user_agent(concat!("hito-server/", env!("CARGO_PKG_VERSION")))
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()?;
    let latest = latest_tag(&client).await?;
    let Some(version) = newer(&latest, &installed) else {
        return Ok(format!(
            "Installed: {installed}; no newer stable release found."
        ));
    };
    if !cfg!(target_os = "linux") {
        return Err("self-update installs Linux releases only".into());
    }
    let arch = arch().ok_or("this architecture has no Hito server package")?;
    let install = detect()?;
    let asset = install.asset(&version, arch);
    let base = format!("https://github.com/{REPO}/releases/download/{latest}");

    // Downloads follow redirects (to GitHub's file storage); the tag lookup above did not.
    let fetcher = reqwest::Client::builder()
        .user_agent(concat!("hito-server/", env!("CARGO_PKG_VERSION")))
        .https_only(true)
        .timeout(Duration::from_secs(600))
        .build()?;
    let sums = fetch(
        &fetcher,
        &format!("{base}/SHA256SUMS"),
        MAX_SUMS_BYTES as u64,
    )
    .await?;
    let sums = String::from_utf8(sums).map_err(|_| "SHA256SUMS is not text")?;
    let expected =
        digest_for(&sums, &asset).ok_or_else(|| format!("SHA256SUMS lists no {asset}"))?;
    let work = tempfile::Builder::new()
        .prefix("hito-update-")
        .tempdir_in(work_root())?;
    let package = work.path().join(&asset);
    let bytes = fetch(&fetcher, &format!("{base}/{asset}"), MAX_PACKAGE_BYTES).await?;
    if hex(&Sha256::digest(&bytes)) != expected {
        return Err(format!(
            "{asset} does not match its SHA-256 in SHA256SUMS; nothing was changed"
        )
        .into());
    }
    std::fs::write(&package, &bytes)?;
    drop(bytes);

    let backup = backup(config, &installed).await?;
    let restore_hint = match &backup {
        Some(path) => format!(" The database was saved first to {}.", path.display()),
        None => String::new(),
    };
    match &install {
        Install::Deb => run_tool("dpkg", &["-i".as_ref(), package.as_os_str()])?,
        Install::Rpm => run_tool("rpm", &["-U".as_ref(), package.as_os_str()])?,
        Install::Binary(exe) => swap_binary(&package, work.path(), exe, &version)?,
    }
    if healthy(config).await {
        if let Install::Binary(exe) = &install {
            let _ = std::fs::remove_file(previous(exe));
        }
        return Ok(format!(
            "Updated from {installed} to {version}.{restore_hint}"
        ));
    }
    if let Install::Binary(exe) = &install {
        std::fs::rename(previous(exe), exe)?;
        restart();
        return Err(format!(
            "{version} did not answer /healthz after the update, so {installed} was put back.{restore_hint}"
        )
        .into());
    }
    Err(format!(
        "{version} is installed but did not answer /healthz; see journalctl -u hito-server.{restore_hint}"
    )
    .into())
}

/// The newest stable release's tag, from where `releases/latest` redirects.
async fn latest_tag(client: &reqwest::Client) -> Result<String, Error> {
    let response = client
        .head(format!("https://github.com/{REPO}/releases/latest"))
        .send()
        .await?;
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .ok_or("GitHub did not say which release is the latest")?;
    tag_from_location(location).ok_or_else(|| "no stable release has been published yet".into())
}

/// `https://github.com/getnodalia/hito/releases/tag/v0.2.0` → `v0.2.0`.
pub fn tag_from_location(location: &str) -> Option<String> {
    let tag = location.strip_prefix(&format!("https://github.com/{REPO}/releases/tag/"))?;
    let ok = !tag.is_empty()
        && tag.len() <= 64
        && tag
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'));
    ok.then(|| tag.to_owned())
}

/// The release's version if it is a stable `vX.Y.Z` newer than `installed`.
pub fn newer(tag: &str, installed: &Version) -> Option<Version> {
    let version = Version::parse(tag.strip_prefix('v')?).ok()?;
    (version.pre.is_empty() && version.build.is_empty() && version > *installed).then_some(version)
}

/// The digest `SHA256SUMS` gives for `name` (`<hex>  <name>` or `<hex> *<name>` lines).
pub fn digest_for(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (digest, file) = line.trim().split_once(char::is_whitespace)?;
        let file = file.trim_start().trim_start_matches('*');
        let ok =
            file == name && digest.len() == 64 && digest.chars().all(|c| c.is_ascii_hexdigit());
        ok.then(|| digest.to_ascii_lowercase())
    })
}

fn arch() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Some("x86_64"),
        "aarch64" => Some("aarch64"),
        _ => None,
    }
}

/// Which package owns the running binary, if any.
fn detect() -> Result<Install, Error> {
    let exe = std::env::current_exe()?.canonicalize()?;
    let owned_by = |tool: &str, args: &[&str]| {
        Command::new(tool)
            .args(args)
            .output()
            .is_ok_and(|o| o.status.success())
    };
    let deb = || {
        Command::new("dpkg-query")
            .args(["-W", "-f=${Status}", "hito-server"])
            .output()
            .is_ok_and(|o| {
                o.status.success() && String::from_utf8_lossy(&o.stdout).contains("ok installed")
            })
    };
    Ok(classify(
        &exe,
        exe == Path::new("/usr/bin/hito-server") && deb(),
        exe == Path::new("/usr/bin/hito-server") && owned_by("rpm", &["-q", "hito-server"]),
    ))
}

pub fn classify(exe: &Path, deb: bool, rpm: bool) -> Install {
    if deb {
        Install::Deb
    } else if rpm {
        Install::Rpm
    } else {
        Install::Binary(exe.to_path_buf())
    }
}

/// Where the download is unpacked: on disk rather than a small tmpfs.
fn work_root() -> PathBuf {
    let var_tmp = PathBuf::from("/var/tmp");
    if var_tmp.is_dir() {
        var_tmp
    } else {
        std::env::temp_dir()
    }
}

async fn fetch(client: &reqwest::Client, url: &str, limit: u64) -> Result<Vec<u8>, Error> {
    let mut response = client.get(url).send().await?.error_for_status()?;
    if response.content_length().is_some_and(|n| n > limit) {
        return Err(format!("{url} is larger than expected").into());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if (body.len() as u64).saturating_add(chunk.len() as u64) > limit {
            return Err(format!("{url} is larger than expected").into());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Snapshots the database into `backups/` beside it, keeping the last few.
/// `None` for a database that is not a file (in memory).
async fn backup(config: &Config, installed: &Version) -> Result<Option<PathBuf>, Error> {
    let Some(db) = database_file(&config.database) else {
        return Ok(None);
    };
    if !db.exists() {
        return Ok(None);
    }
    let dir = db
        .parent()
        .map(|p| p.join("backups"))
        .ok_or("the database has no folder")?;
    std::fs::create_dir_all(&dir)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let dest = dir.join(format!("before-update-{installed}-{stamp}.db"));
    hito_storage::backup::create(&config.database, &dest).await?;
    prune(&dir);
    Ok(Some(dest))
}

/// Removes all but the newest [`KEEP_BACKUPS`] pre-update snapshots.
fn prune(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut snapshots: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("before-update-") && n.ends_with(".db"))
        })
        .collect();
    snapshots.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
    let excess = snapshots.len().saturating_sub(KEEP_BACKUPS);
    for old in snapshots.into_iter().take(excess) {
        let _ = std::fs::remove_file(old);
    }
}

/// `sqlite:///var/lib/hito/hito.db` → `/var/lib/hito/hito.db`; `sqlite://hito.db` → a
/// relative path; anything in memory → `None`.
pub fn database_file(url: &str) -> Option<PathBuf> {
    if url.contains(":memory:") {
        return None;
    }
    let path = url
        .strip_prefix("sqlite://")
        .or_else(|| url.strip_prefix("sqlite:"))?;
    let path = path.split('?').next().unwrap_or_default();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

fn run_tool(tool: &str, args: &[&std::ffi::OsStr]) -> Result<(), Error> {
    let status = Command::new(tool).args(args).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{tool} failed ({status}); the previous version stays installed").into())
    }
}

fn previous(exe: &Path) -> PathBuf {
    let mut name = exe.as_os_str().to_owned();
    name.push(".previous");
    PathBuf::from(name)
}

/// Unpacks the `.tar.gz`, checks the new binary runs and is the expected version, then puts it in
/// place (the old one kept as `<exe>.previous`) and restarts the service.
fn swap_binary(package: &Path, work: &Path, exe: &Path, version: &Version) -> Result<(), Error> {
    let unpacked = work.join("unpacked");
    std::fs::create_dir_all(&unpacked)?;
    run_tool(
        "tar",
        &[
            "-xzf".as_ref(),
            package.as_os_str(),
            "-C".as_ref(),
            unpacked.as_os_str(),
        ],
    )?;
    let new = unpacked
        .join(format!(
            "hito-server-{version}-linux-{}",
            arch().unwrap_or_default()
        ))
        .join("hito-server");
    let output = Command::new(&new).arg("--version").output()?;
    let said = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() || said.trim() != format!("hito-server {version}") {
        return Err(format!(
            "the downloaded server does not run as {version}; nothing was changed"
        )
        .into());
    }
    let dir = exe.parent().ok_or("the server has no folder")?;
    let staged = dir.join(".hito-server.new");
    let _ = std::fs::remove_file(&staged);
    std::fs::copy(&new, &staged)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
    }
    let _ = std::fs::remove_file(previous(exe));
    std::fs::hard_link(exe, previous(exe))
        .or_else(|_| std::fs::copy(exe, previous(exe)).map(|_| ()))?;
    std::fs::rename(&staged, exe)?;
    restart();
    Ok(())
}

fn restart() {
    let _ = Command::new("systemctl")
        .args(["try-restart", "hito-server.service"])
        .status();
}

/// Whether the server answers `/healthz` within a minute.
async fn healthy(config: &Config) -> bool {
    let mut address = config.bind;
    if address.ip().is_unspecified() {
        address.set_ip(if address.is_ipv4() {
            std::net::Ipv4Addr::LOCALHOST.into()
        } else {
            std::net::Ipv6Addr::LOCALHOST.into()
        });
    }
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
    else {
        return false;
    };
    let url = format!("http://{address}/healthz");
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_secs(2)).await;
        if client
            .get(&url)
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
        {
            return true;
        }
    }
    false
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_latest_tag_comes_from_hito_s_release_page_only() {
        assert_eq!(
            tag_from_location("https://github.com/getnodalia/hito/releases/tag/v0.2.0"),
            Some("v0.2.0".into())
        );
        for location in [
            "https://github.com/getnodalia/hito/releases",
            "https://github.com/someone/hito/releases/tag/v9.0.0",
            "https://github.com/getnodalia/hito/releases/tag/v1/../x",
            "https://github.com/getnodalia/hito/releases/tag/",
        ] {
            assert_eq!(tag_from_location(location), None, "{location}");
        }
    }

    #[test]
    fn only_a_newer_stable_version_is_installed() {
        let installed = Version::new(0, 1, 4);
        assert_eq!(newer("v0.1.5", &installed), Some(Version::new(0, 1, 5)));
        assert_eq!(newer("v0.10.0", &installed), Some(Version::new(0, 10, 0)));
        for tag in ["v0.1.4", "v0.1.3", "v0.2.0-rc.1", "0.2.0", "vx"] {
            assert_eq!(newer(tag, &installed), None, "{tag}");
        }
    }

    #[test]
    fn the_package_matches_how_the_server_was_installed() {
        let v = Version::new(0, 2, 0);
        assert_eq!(
            Install::Deb.asset(&v, "x86_64"),
            "hito-server-0.2.0-linux-x86_64.deb"
        );
        assert_eq!(
            Install::Rpm.asset(&v, "aarch64"),
            "hito-server-0.2.0-linux-aarch64.rpm"
        );
        let exe = Path::new("/usr/local/bin/hito-server");
        assert_eq!(
            classify(exe, false, false).asset(&v, "x86_64"),
            "hito-server-0.2.0-linux-x86_64.tar.gz"
        );
        assert_eq!(classify(exe, true, false), Install::Deb);
        assert_eq!(classify(exe, false, true), Install::Rpm);
        assert_eq!(
            classify(exe, false, false),
            Install::Binary(exe.to_path_buf())
        );
    }

    #[test]
    fn sha256sums_gives_the_digest_of_exactly_that_file() {
        let a = "a".repeat(64);
        let b = "B".repeat(64);
        let sums = format!(
            "{a}  hito-server-0.2.0-linux-x86_64.deb\n{b} *hito-server-0.2.0-linux-x86_64.tar.gz\n"
        );
        assert_eq!(
            digest_for(&sums, "hito-server-0.2.0-linux-x86_64.deb"),
            Some(a.clone())
        );
        assert_eq!(
            digest_for(&sums, "hito-server-0.2.0-linux-x86_64.tar.gz"),
            Some("b".repeat(64))
        );
        assert_eq!(
            digest_for(&sums, "hito-server-0.2.0-linux-x86_64.rpm"),
            None
        );
        assert_eq!(digest_for("short  hito.deb", "hito.deb"), None);
    }

    #[test]
    fn the_database_file_comes_from_its_url() {
        assert_eq!(
            database_file("sqlite:///var/lib/hito/hito.db"),
            Some(PathBuf::from("/var/lib/hito/hito.db"))
        );
        assert_eq!(
            database_file("sqlite://hito.db?mode=rwc"),
            Some(PathBuf::from("hito.db"))
        );
        assert_eq!(database_file("sqlite::memory:"), None);
        assert_eq!(database_file("postgres://x"), None);
    }

    #[test]
    fn only_the_newest_pre_update_backups_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..5 {
            let path = dir.path().join(format!("before-update-0.1.{i}-{i}.db"));
            std::fs::write(&path, b"x").unwrap();
            let time = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000 + i);
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(time)
                .unwrap();
        }
        std::fs::write(dir.path().join("manual.db"), b"x").unwrap();
        prune(dir.path());
        let mut left: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                "before-update-0.1.2-2.db",
                "before-update-0.1.3-3.db",
                "before-update-0.1.4-4.db",
                "manual.db",
            ]
        );
    }
}
