//! End-to-end encrypted projects on top of [`Client`].
//!
//! [`Secure`] owns this device's [`hito_e2e::Device`] and does what every front end needs: it
//! registers the device with the session, keeps each project's MLS group in step with the
//! server's log ([`Secure::sync`]), hands the project's keys to devices that join and makes a new
//! key when devices are removed, seals what is written (task content, milestones, comments,
//! descriptions) and opens what the server returns. All MLS work for one device runs under one
//! lock, and the device file is rewritten (owner-only, atomically) after each change.
//!
//! The server sees a sealed text's key id and length, never its contents.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use hito_core::{validate, ProjectId, UserId};
use hito_e2e::{Device, Entry, Field, Leaf, Outcome};
use hito_protocol::{
    Activity, ApiError, Comment, CommitRequest, ErrorCode, Milestone, MilestoneContent, MlsState,
    Project, Sealed, Task, TaskContent,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::sync::Mutex;

use crate::{Client, ClientError, Result};

pub use hito_e2e::Field as SealedField;

/// Attempts at an operation that keeps losing the race to other devices' commits.
const ATTEMPTS: usize = 6;
/// Publish more key packages when fewer than this are left on the server...
const KEY_PACKAGES_LOW: u32 = 20;
/// ...this many at a time.
const KEY_PACKAGES_BATCH: usize = 50;

fn crypto(e: hito_e2e::Error) -> ClientError {
    ClientError::Encryption(e.to_string())
}

/// A validation failure, reported the way the server would have before the text was sealed.
fn invalid(e: hito_core::ValidationError) -> ClientError {
    ClientError::Api {
        status: 400,
        error: ApiError {
            code: ErrorCode::Invalid,
            message: e.to_string(),
            field: Some(e.field().to_owned()),
        },
    }
}

fn stale(e: &ClientError) -> bool {
    matches!(e.code(), Some(ErrorCode::StaleEpoch | ErrorCode::Conflict))
}

/// Check and tidy a task's content before it is sealed and out of the server's sight.
pub fn checked_task(content: &TaskContent) -> Result<TaskContent> {
    let mut checklist = Vec::with_capacity(content.checklist.len());
    if content.checklist.len() > hito_core::limits::CHECKLIST_MAX {
        return Err(invalid(hito_core::ValidationError::TooLong {
            field: "checklist",
            max: hito_core::limits::CHECKLIST_MAX,
        }));
    }
    for item in &content.checklist {
        checklist.push(hito_protocol::ChecklistItem {
            text: validate::checklist_item(&item.text).map_err(invalid)?,
            done: item.done,
        });
    }
    Ok(TaskContent {
        title: validate::task_title(&content.title).map_err(invalid)?,
        description: validate::description(&content.description).map_err(invalid)?,
        labels: validate::labels(&content.labels).map_err(invalid)?,
        checklist,
    })
}

pub fn checked_milestone(content: &MilestoneContent) -> Result<MilestoneContent> {
    Ok(MilestoneContent {
        name: validate::milestone_name(&content.name).map_err(invalid)?,
        description: validate::description(&content.description).map_err(invalid)?,
    })
}

/// Something a sync learned about a project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// New project keys: text this device could not open before may open now.
    Keys,
    /// Devices joined or left the project.
    Devices { joined: Vec<Leaf>, left: Vec<Leaf> },
}

struct Inner {
    device: Device,
    path: Option<PathBuf>,
}

impl Inner {
    fn save(&self) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let bytes = self.device.to_bytes().map_err(crypto)?;
        write_private(path, &bytes).map_err(|e| ClientError::Encryption(e.to_string()))
    }
}

/// Write `bytes` to `path` readable by its owner only, replacing it atomically. The folder is
/// created when missing: on a first launch nothing has made the settings folder yet.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

/// Keep enough key packages on the server for others to add this device while it is away.
async fn replenish(client: &Client, inner: &mut Inner, left: u32) -> Result<()> {
    if left >= KEY_PACKAGES_LOW {
        return Ok(());
    }
    let packages = inner
        .device
        .key_packages(KEY_PACKAGES_BATCH)
        .map_err(crypto)?;
    // Save the private halves before the public ones are out.
    inner.save()?;
    client
        .publish_key_packages(packages.iter().map(|p| STANDARD.encode(p)).collect())
        .await?;
    Ok(())
}

/// This device's end-to-end encryption, for one signed-in session.
#[derive(Clone)]
pub struct Secure {
    client: Client,
    inner: Arc<Mutex<Inner>>,
    fingerprint: String,
    user: UserId,
}

impl std::fmt::Debug for Secure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Secure")
            .field("user", &self.user)
            .field("fingerprint", &self.fingerprint)
            .finish_non_exhaustive()
    }
}

impl Secure {
    /// Load this device from `path` (or create it), and register it with `client`'s session.
    /// Without a path the device lives in memory only, and a restart makes a new one.
    pub async fn open(client: &Client, user: UserId, path: Option<PathBuf>) -> Result<Secure> {
        let saved = path
            .as_deref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|bytes| Device::from_bytes(&bytes).ok())
            .filter(|d| d.user() == user);
        let device = match saved {
            Some(d) => d,
            None => Device::new(user).map_err(crypto)?,
        };
        let fingerprint = device.fingerprint();
        let mut inner = Inner { device, path };
        let status = client
            .register_device(&STANDARD.encode(inner.device.public_key()))
            .await?;
        replenish(client, &mut inner, status.key_packages).await?;
        inner.save()?;
        Ok(Secure {
            client: client.clone(),
            inner: Arc::new(Mutex::new(inner)),
            fingerprint,
            user,
        })
    }

    /// Delete a device file (on logout: its keys).
    pub fn wipe(path: &Path) {
        let _ = std::fs::remove_file(path);
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    /// This device's fingerprint, for people to compare.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// The devices in a project's group, as this device sees them.
    pub async fn members(&self, project: ProjectId) -> Vec<Leaf> {
        self.inner.lock().await.device.members(project)
    }

    /// Whether this device holds a key for `project` (it can open and seal its text).
    pub async fn has_keys(&self, project: ProjectId) -> bool {
        self.inner.lock().await.device.has_keys(project)
    }

    /// Bring this device up to date with a project: join its group if needed, read every new
    /// log entry in order, and hand the project's keys to devices that joined.
    pub async fn sync(&self, project: ProjectId) -> Result<Vec<Change>> {
        let mut inner = self.inner.lock().await;
        let (changes, _) = self.sync_locked(&mut inner, project).await?;
        inner.save()?;
        Ok(changes)
    }

    async fn sync_locked(
        &self,
        inner: &mut Inner,
        project: ProjectId,
    ) -> Result<(Vec<Change>, MlsState)> {
        let mut changes = Vec::new();
        for _ in 0..ATTEMPTS {
            if !inner.device.is_member(project) {
                self.enter_locked(inner, project).await?;
            }
            let mut joined_since = false;
            let state = loop {
                let after = inner.device.last_seq(project);
                let state = self.client.mls_state(project, Some(after)).await?;
                let mut removed = false;
                for e in &state.entries {
                    let entry = Entry {
                        seq: e.seq,
                        author: e.author_id,
                        data: STANDARD.decode(&e.data).unwrap_or_default(),
                    };
                    match inner.device.process(project, &entry).map_err(crypto)? {
                        Outcome::Keys { new } if new > 0 => changes.push(Change::Keys),
                        Outcome::Changed { joined, left } => {
                            joined_since |= !joined.is_empty();
                            changes.push(Change::Devices { joined, left });
                        }
                        Outcome::Removed => {
                            removed = true;
                            break;
                        }
                        Outcome::Keys { .. } | Outcome::Skipped | Outcome::Unreadable => {}
                    }
                }
                if removed {
                    break None;
                }
                if !state.has_more {
                    break Some(state);
                }
            };
            let Some(state) = state else {
                // Removed from the group (a stale device, or a guest added again): the server
                // still lets us read, so join again from here on. Known keys stay.
                inner.device.forget(project).map_err(crypto)?;
                continue;
            };
            if joined_since && inner.device.has_keys(project) {
                // New devices cannot read anything until a member hands them the keys.
                if !self.share_locked(inner, project, false).await? {
                    continue;
                }
            }
            return Ok((changes, state));
        }
        Err(ClientError::Encryption(
            "the project's encryption keeps changing; try again".into(),
        ))
    }

    /// Send the project's keys to its group, after making a new one when `rotate`. `false` when
    /// another commit got in first (catch up and try again).
    async fn share_locked(
        &self,
        inner: &mut Inner,
        project: ProjectId,
        rotate: bool,
    ) -> Result<bool> {
        let (data, _) = if rotate {
            inner.device.rotate(project)
        } else {
            inner.device.share_keys(project)
        }
        .map_err(crypto)?;
        // A new key must survive a crash before anything is sealed with it.
        inner.save()?;
        match self
            .client
            .mls_share_keys(project, &STANDARD.encode(data))
            .await
        {
            Ok(res) => {
                inner.device.sent(project, res.seq);
                Ok(true)
            }
            Err(e) if stale(&e) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Enter `project`'s group: through a Welcome someone left for this device, or else by
    /// joining on its own.
    async fn enter_locked(&self, inner: &mut Inner, project: ProjectId) -> Result<()> {
        let state = self.client.mls_state(project, None).await?;
        if let Some(welcome) = &state.welcome {
            if let Ok(data) = STANDARD.decode(&welcome.data) {
                if inner.device.welcome(project, &data, welcome.seq).is_ok() {
                    // That used one of this device's key packages.
                    let status = self
                        .client
                        .register_device(&STANDARD.encode(inner.device.public_key()))
                        .await?;
                    return replenish(&self.client, inner, status.key_packages).await;
                }
            }
        }
        self.join_locked(inner, project).await
    }

    /// Start or join `project`'s group. The device that starts it makes the first key.
    async fn join_locked(&self, inner: &mut Inner, project: ProjectId) -> Result<()> {
        for _ in 0..ATTEMPTS {
            let state = self.client.mls_state(project, None).await?;
            match &state.group_info {
                None => {
                    let info = inner
                        .device
                        .create(project, state.last_seq)
                        .map_err(crypto)?;
                    match self
                        .client
                        .mls_create_group(project, &STANDARD.encode(info))
                        .await
                    {
                        Ok(_) => {
                            inner.device.confirm(project, None).map_err(crypto)?;
                            if !inner.device.has_keys(project) {
                                self.share_locked(inner, project, true).await?;
                            }
                            return Ok(());
                        }
                        Err(e) if stale(&e) => inner.device.abort(project).map_err(crypto)?,
                        Err(e) => {
                            inner.device.abort(project).map_err(crypto)?;
                            return Err(e);
                        }
                    }
                }
                Some(info) => {
                    let info = STANDARD
                        .decode(info)
                        .map_err(|e| ClientError::Decode(e.to_string()))?;
                    let pending = inner
                        .device
                        .join(project, &info, state.last_seq)
                        .map_err(crypto)?;
                    match self.commit(project, &pending).await {
                        Ok(seq) => {
                            inner.device.confirm(project, Some(seq)).map_err(crypto)?;
                            return Ok(());
                        }
                        Err(e) if stale(&e) => inner.device.abort(project).map_err(crypto)?,
                        Err(e) => {
                            inner.device.abort(project).map_err(crypto)?;
                            return Err(e);
                        }
                    }
                }
            }
        }
        Err(ClientError::Encryption(
            "could not join the project's encryption; try again".into(),
        ))
    }

    async fn commit(&self, project: ProjectId, pending: &hito_e2e::Pending) -> Result<i64> {
        let res = self
            .client
            .mls_commit(
                project,
                &CommitRequest {
                    commit: STANDARD.encode(&pending.commit),
                    group_info: STANDARD.encode(&pending.group_info),
                    welcome: None,
                    welcome_to: Vec::new(),
                },
            )
            .await?;
        Ok(res.seq)
    }

    /// Catch up, remove devices that may no longer read the project (and make a new key they
    /// never see), and add the ones that may but are not in it yet (and hand them the keys).
    async fn prepare_locked(&self, inner: &mut Inner, project: ProjectId) -> Result<()> {
        for _ in 0..ATTEMPTS {
            if self.remove_locked(inner, project).await? && self.add_locked(inner, project).await? {
                if !inner.device.has_keys(project) && self.alone(inner, project) {
                    // Nobody is left to hand this device a key (the device that started the
                    // project never shared one, or every other device is gone): make one.
                    if !self.share_locked(inner, project, true).await? {
                        continue;
                    }
                }
                return Ok(());
            }
        }
        Err(ClientError::Encryption(
            "the project's encryption keeps changing; try again".into(),
        ))
    }

    fn alone(&self, inner: &Inner, project: ProjectId) -> bool {
        let me = inner.device.public_key();
        inner
            .device
            .members(project)
            .iter()
            .all(|l| l.public_key == me)
    }

    /// Add listed devices missing from the group. `false` if another commit got in first.
    async fn add_locked(&self, inner: &mut Inner, project: ProjectId) -> Result<bool> {
        let (_, state) = self.sync_locked(inner, project).await?;
        let present: HashSet<Vec<u8>> = inner
            .device
            .members(project)
            .into_iter()
            .map(|l| l.public_key)
            .collect();
        let missing: Vec<String> = state
            .devices
            .iter()
            .filter(|d| {
                STANDARD
                    .decode(&d.public_key)
                    .is_ok_and(|k| !present.contains(&k))
            })
            .map(|d| d.public_key.clone())
            .collect();
        if missing.is_empty() {
            return Ok(true);
        }
        let claimed = self.client.claim_key_packages(project, missing).await?;
        let packages: Vec<(Leaf, Vec<u8>)> = claimed
            .iter()
            .filter_map(|c| {
                Some((
                    Leaf {
                        user: c.device.user_id,
                        public_key: STANDARD.decode(&c.device.public_key).ok()?,
                    },
                    STANDARD.decode(&c.key_package).ok()?,
                ))
            })
            .collect();
        let Some((pending, welcome)) = inner.device.add(project, &packages).map_err(crypto)? else {
            return Ok(true);
        };
        let req = CommitRequest {
            commit: STANDARD.encode(&pending.commit),
            group_info: STANDARD.encode(&pending.group_info),
            welcome: Some(STANDARD.encode(welcome)),
            welcome_to: claimed
                .iter()
                .map(|c| c.device.public_key.clone())
                .collect(),
        };
        match self.client.mls_commit(project, &req).await {
            Ok(res) => {
                inner
                    .device
                    .confirm(project, Some(res.seq))
                    .map_err(crypto)?;
                if inner.device.has_keys(project) {
                    // The devices just added start reading here: hand them every key. If this
                    // races another commit, the next sync shares them instead.
                    self.share_locked(inner, project, false).await?;
                }
                Ok(true)
            }
            Err(e) if stale(&e) => {
                inner.device.abort(project).map_err(crypto)?;
                Ok(false)
            }
            Err(e) => {
                inner.device.abort(project).map_err(crypto)?;
                Err(e)
            }
        }
    }

    /// Remove devices that may no longer read, then make a new key. `false` if another commit
    /// got in first.
    async fn remove_locked(&self, inner: &mut Inner, project: ProjectId) -> Result<bool> {
        let (_, state) = self.sync_locked(inner, project).await?;
        let allowed: HashSet<(UserId, Vec<u8>)> = state
            .devices
            .iter()
            .filter_map(|d| Some((d.user_id, STANDARD.decode(&d.public_key).ok()?)))
            .collect();
        let pending = inner
            .device
            .remove(project, |leaf| {
                allowed.contains(&(leaf.user, leaf.public_key.clone()))
            })
            .map_err(crypto)?;
        let Some(pending) = pending else {
            return Ok(true);
        };
        match self.commit(project, &pending).await {
            Ok(seq) => {
                inner.device.confirm(project, Some(seq)).map_err(crypto)?;
                // Those devices keep the old keys; what is written from now on uses a new one.
                self.share_locked(inner, project, true).await
            }
            Err(e) if stale(&e) => {
                inner.device.abort(project).map_err(crypto)?;
                Ok(false)
            }
            Err(e) => {
                inner.device.abort(project).map_err(crypto)?;
                Err(e)
            }
        }
    }

    /// Seal `text` for `project` with its newest key, after bringing the group up to date.
    pub async fn seal_text(&self, project: ProjectId, field: Field, text: &str) -> Result<Sealed> {
        let mut inner = self.inner.lock().await;
        self.prepare_locked(&mut inner, project).await?;
        let sealed = inner
            .device
            .seal(project, field, text)
            .map_err(|e| match e {
                hito_e2e::Error::NoKey => ClientError::Encryption(
                    "this device is waiting for the project's keys from a teammate's device".into(),
                ),
                other => crypto(other),
            });
        inner.save()?;
        let sealed = sealed?;
        Ok(Sealed {
            key_id: sealed.key_id,
            data: STANDARD.encode(sealed.data),
        })
    }

    pub async fn seal_json<T: Serialize>(
        &self,
        project: ProjectId,
        field: Field,
        value: &T,
    ) -> Result<Sealed> {
        let text = serde_json::to_string(value).map_err(|e| ClientError::Decode(e.to_string()))?;
        self.seal_text(project, field, &text).await
    }

    /// Seal a task's content, after checking it.
    pub async fn seal_task(&self, project: ProjectId, content: &TaskContent) -> Result<Sealed> {
        self.seal_json(project, Field::Task, &checked_task(content)?)
            .await
    }

    pub async fn seal_milestone(
        &self,
        project: ProjectId,
        content: &MilestoneContent,
    ) -> Result<Sealed> {
        self.seal_json(project, Field::Milestone, &checked_milestone(content)?)
            .await
    }

    pub async fn seal_comment(&self, project: ProjectId, body: &str) -> Result<Sealed> {
        let body = validate::comment_body(body).map_err(invalid)?;
        self.seal_text(project, Field::Comment, &body).await
    }

    pub async fn seal_description(&self, project: ProjectId, text: &str) -> Result<Sealed> {
        let text = validate::description(text).map_err(invalid)?;
        self.seal_text(project, Field::Project, &text).await
    }

    /// Open a sealed text with the keys this device holds. `None` when it cannot (no key yet,
    /// or the text was tampered with).
    pub async fn open_text(
        &self,
        project: ProjectId,
        field: Field,
        sealed: &Sealed,
    ) -> Option<String> {
        if sealed.key_id == 0 {
            return None;
        }
        let data = STANDARD.decode(&sealed.data).ok()?;
        let inner = self.inner.lock().await;
        inner
            .device
            .open(
                project,
                field,
                &hito_e2e::Sealed {
                    key_id: sealed.key_id,
                    data,
                },
            )
            .ok()
    }

    pub async fn open_json<T: DeserializeOwned>(
        &self,
        project: ProjectId,
        field: Field,
        sealed: &Sealed,
    ) -> Option<T> {
        let text = self.open_text(project, field, sealed).await?;
        serde_json::from_str(&text).ok()
    }

    pub async fn task_content(&self, task: &Task) -> Option<TaskContent> {
        self.open_json(task.project_id, Field::Task, &task.content)
            .await
    }

    pub async fn milestone_content(&self, milestone: &Milestone) -> Option<MilestoneContent> {
        self.open_json(milestone.project_id, Field::Milestone, &milestone.content)
            .await
    }

    pub async fn comment_text(&self, project: ProjectId, comment: &Comment) -> Option<String> {
        self.open_text(project, Field::Comment, &comment.body).await
    }

    pub async fn description(&self, project: &Project) -> Option<String> {
        let sealed = project.description.as_ref()?;
        self.open_text(project.id, Field::Project, sealed).await
    }

    /// What an activity entry was about: a task's title or a milestone's name, as it was then.
    pub async fn activity_subject(&self, activity: &Activity) -> Option<String> {
        let sealed = activity.subject.as_ref()?;
        if activity.task_id.is_some() {
            self.open_json::<TaskContent>(activity.project_id, Field::Task, sealed)
                .await
                .map(|c| c.title)
        } else {
            self.open_json::<MilestoneContent>(activity.project_id, Field::Milestone, sealed)
                .await
                .map(|c| c.name)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_device_file_is_written_into_a_folder_that_does_not_exist_yet() {
        let root = std::env::temp_dir().join(format!("hito-secure-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("first-launch").join("device.json");
        write_private(&path, b"keys").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"keys");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn task_content_is_checked_before_it_is_sealed() {
        let ok = checked_task(&TaskContent {
            title: "  Revisar el presupuesto ".into(),
            labels: vec!["UX".into(), "UX".into()],
            ..TaskContent::default()
        })
        .unwrap();
        assert_eq!(ok.title, "Revisar el presupuesto");
        assert_eq!(ok.labels, vec!["UX".to_owned()]);
        let empty = checked_task(&TaskContent::default()).unwrap_err();
        assert_eq!(empty.code(), Some(ErrorCode::Invalid));
    }
}
