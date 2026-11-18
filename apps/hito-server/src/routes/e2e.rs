//! End-to-end encryption: devices, each project's MLS group and its encrypted log.
//!
//! The server cannot decrypt anything here. What it does check is everything it can see without
//! keys: that the caller may read the project, that an MLS message belongs to this
//! project's group (by its group id) and to its current epoch, and that commits arrive one
//! epoch at a time. The order it assigns (`seq`) is the order every device processes.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use hito_core::{ProjectId, UserId};
use hito_e2e::Kind;
use hito_protocol::{
    ClaimKeyPackagesRequest, ClaimedKeyPackage, CommitRequest, CommitResponse, CreateGroupRequest,
    Device, DeviceStatus, MlsEntry, MlsState, MlsWelcome, PublishKeyPackagesRequest,
    RegisterDeviceRequest, ServerEvent, ShareKeysRequest,
};
use serde::Deserialize;

use super::{check_write, project_access};
use crate::auth::{now_ms, Auth};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// Log entries returned per request.
const LOG_PAGE_MAX: u32 = 500;
/// Ed25519 public keys are 32 bytes; leave room for other suites later.
const PUBLIC_KEY_MAX: usize = 256;
/// Unused key packages one device may keep on the server.
const KEY_PACKAGES_MAX: i64 = 200;
/// A key package is a few hundred bytes.
const KEY_PACKAGE_BYTES_MAX: usize = 4_096;
/// Devices one request may claim key packages for (and one Welcome may address).
const DEVICES_PER_REQUEST_MAX: usize = 500;

pub(crate) fn decode(field: &str, value: &str) -> AppResult<Vec<u8>> {
    let bytes = STANDARD.decode(value).map_err(|_| {
        let mut e = AppError::invalid("not valid base64");
        e.body.field = Some(field.to_owned());
        e
    })?;
    if bytes.is_empty() {
        let mut e = AppError::invalid("must not be empty");
        e.body.field = Some(field.to_owned());
        return Err(e);
    }
    Ok(bytes)
}

/// Check an MLS message is of `kind` and belongs to `project`'s group; return its epoch.
pub(crate) fn check_mls(project: ProjectId, bytes: &[u8], kind: Kind) -> AppResult<u64> {
    let header = hito_e2e::inspect(bytes).map_err(|_| AppError::invalid("not an MLS message"))?;
    if header.group_id != hito_e2e::group_id(project) {
        return Err(AppError::invalid(
            "the MLS message belongs to another project",
        ));
    }
    if header.kind != kind {
        return Err(AppError::invalid("unexpected kind of MLS message"));
    }
    Ok(header.epoch)
}

fn check_group_info(project: ProjectId, bytes: &[u8]) -> AppResult<u64> {
    let (group, epoch) = hito_e2e::inspect_group_info(bytes)
        .map_err(|_| AppError::invalid("not an MLS GroupInfo"))?;
    if group != hito_e2e::group_id(project) {
        return Err(AppError::invalid(
            "the GroupInfo belongs to another project",
        ));
    }
    Ok(epoch)
}

async fn device_status(state: &AppState, key: &[u8]) -> AppResult<Json<DeviceStatus>> {
    let count = state.store.key_package_count(key).await?;
    Ok(Json(DeviceStatus {
        key_packages: u32::try_from(count).unwrap_or(0),
    }))
}

/// `PUT /devices/me`: the caller's session's MLS signing key.
pub async fn register_device(
    State(state): State<AppState>,
    auth: Auth,
    Json(req): Json<RegisterDeviceRequest>,
) -> AppResult<Json<DeviceStatus>> {
    let key = decode("public_key", &req.public_key)?;
    if key.len() > PUBLIC_KEY_MAX {
        return Err(AppError::invalid("public_key is too long"));
    }
    state
        .store
        .register_device(auth.session()?, auth.user.id, &key, now_ms())
        .await
        .map_err(|e| match e {
            hito_storage::StoreError::Conflict => {
                AppError::conflict("this key belongs to another account")
            }
            other => other.into(),
        })?;
    device_status(&state, &key).await
}

/// The caller's registered device, or an error telling the client to register it first.
async fn own_device(state: &AppState, auth: &Auth) -> AppResult<Vec<u8>> {
    state
        .store
        .session_device(auth.session()?)
        .await?
        .ok_or_else(|| AppError::invalid("register this session's device first"))
}

/// `POST /devices/me/key_packages`: publish key packages for the caller's device. Each must be
/// signed by that device for that account.
pub async fn publish_key_packages(
    State(state): State<AppState>,
    auth: Auth,
    Json(req): Json<PublishKeyPackagesRequest>,
) -> AppResult<Json<DeviceStatus>> {
    let key = own_device(&state, &auth).await?;
    let have = state.store.key_package_count(&key).await?;
    let room = usize::try_from(KEY_PACKAGES_MAX.saturating_sub(have)).unwrap_or(0);
    if req.key_packages.len() > room {
        return Err(AppError::invalid("too many key packages; publish fewer"));
    }
    let mut packages = Vec::with_capacity(req.key_packages.len());
    for kp in &req.key_packages {
        if kp.len() > KEY_PACKAGE_BYTES_MAX * 2 {
            return Err(AppError::invalid("a key package is too large"));
        }
        let bytes = decode("key_packages", kp)?;
        let leaf = hito_e2e::inspect_key_package(&bytes)
            .map_err(|_| AppError::invalid("not a valid key package"))?;
        if leaf.user != auth.user.id || leaf.public_key != key {
            return Err(AppError::invalid("a key package belongs to another device"));
        }
        packages.push(bytes);
    }
    state
        .store
        .add_key_packages(&key, &packages, now_ms())
        .await?;
    device_status(&state, &key).await
}

/// The devices of people who may read a project, as (user, key).
async fn allowed_devices(
    state: &AppState,
    project: ProjectId,
) -> AppResult<Vec<(UserId, Vec<u8>)>> {
    let audience = state.store.project_audience(project).await?;
    Ok(state.store.devices(&audience, now_ms()).await?)
}

/// `POST /projects/{id}/mls/key_packages`: take one key package for each listed device of
/// someone who may read the project, to add those devices to its group.
pub async fn claim_key_packages(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
    Json(req): Json<ClaimKeyPackagesRequest>,
) -> AppResult<Json<Vec<ClaimedKeyPackage>>> {
    let access = project_access(&state, ProjectId(id), auth.user.id).await?;
    check_write(&state, auth.user.id)?;
    if req.devices.len() > DEVICES_PER_REQUEST_MAX {
        return Err(AppError::invalid("too many devices"));
    }
    let allowed = allowed_devices(&state, access.id()).await?;
    let mut out = Vec::new();
    for wanted in &req.devices {
        let key = decode("devices", wanted)?;
        let Some((user, _)) = allowed.iter().find(|(_, k)| *k == key) else {
            continue;
        };
        if let Some(kp) = state.store.claim_key_package(&key).await? {
            out.push(ClaimedKeyPackage {
                device: Device {
                    user_id: *user,
                    public_key: wanted.clone(),
                },
                key_package: STANDARD.encode(kp),
            });
        }
    }
    Ok(Json(out))
}

#[derive(Debug, Deserialize)]
pub struct LogQuery {
    after: Option<i64>,
    limit: Option<u32>,
}

/// `GET /projects/{id}/mls`: the group's state, the devices that may be in it, and with
/// `after`, the log entries that follow.
pub async fn state(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
    Query(q): Query<LogQuery>,
) -> AppResult<Json<MlsState>> {
    let access = project_access(&state, ProjectId(id), auth.user.id).await?;
    let project = access.id();
    let group = state.store.mls_group(project).await?;
    let welcome = match state.store.session_device(auth.session()?).await? {
        Some(key) => state.store.welcome(project, &key).await?,
        None => None,
    };
    let devices = allowed_devices(&state, access.id())
        .await?
        .into_iter()
        .map(|(user_id, key)| Device {
            user_id,
            public_key: STANDARD.encode(key),
        })
        .collect();
    let last_seq = state.store.mls_last_seq(project).await?;
    let (entries, has_more) = match q.after {
        Some(after) => {
            let limit = q.limit.unwrap_or(LOG_PAGE_MAX).clamp(1, LOG_PAGE_MAX);
            state.store.mls_log(project, after.max(0), limit).await?
        }
        None => (Vec::new(), false),
    };
    let entries = entries
        .into_iter()
        .map(|e| MlsEntry {
            seq: e.seq,
            epoch: e.epoch,
            kind: e.kind,
            author_id: e.author,
            data: STANDARD.encode(e.data),
        })
        .collect();
    Ok(Json(MlsState {
        project_id: project,
        epoch: group.as_ref().map(|g| g.0),
        group_info: group.map(|g| STANDARD.encode(g.1)),
        last_seq,
        devices,
        entries,
        has_more,
        welcome: welcome.map(|(seq, data)| MlsWelcome {
            seq,
            data: STANDARD.encode(data),
        }),
    }))
}

pub(crate) async fn announce(state: &AppState, project: ProjectId) -> AppResult<()> {
    let audience: Vec<UserId> = state.store.project_audience(project).await?;
    state.hub.send(
        &audience,
        &ServerEvent::MlsChanged {
            project_id: project,
        },
    );
    Ok(())
}

/// `POST /projects/{id}/mls/group`: start the project's group. Only the first device to
/// ask gets to; the others join it.
pub async fn create_group(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
    Json(req): Json<CreateGroupRequest>,
) -> AppResult<(StatusCode, Json<CommitResponse>)> {
    let access = project_access(&state, ProjectId(id), auth.user.id).await?;
    check_write(&state, auth.user.id)?;
    let info = decode("group_info", &req.group_info)?;
    let epoch = check_group_info(access.id(), &info)?;
    if epoch != 0 {
        return Err(AppError::invalid("a new group starts at epoch 0"));
    }
    state
        .store
        .create_mls_group(access.id(), epoch, &info, now_ms())
        .await
        .map_err(|e| match e {
            hito_storage::StoreError::Conflict => {
                AppError::conflict("this project already has a group; join it")
            }
            other => other.into(),
        })?;
    announce(&state, access.id()).await?;
    let seq = state.store.mls_last_seq(access.id()).await?;
    Ok((StatusCode::CREATED, Json(CommitResponse { seq, epoch })))
}

/// `POST /projects/{id}/mls/commit`: a commit for the current epoch (a device joining, or
/// someone removing devices that may no longer read).
pub async fn commit(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
    Json(req): Json<CommitRequest>,
) -> AppResult<(StatusCode, Json<CommitResponse>)> {
    let access = project_access(&state, ProjectId(id), auth.user.id).await?;
    check_write(&state, auth.user.id)?;
    let project = access.id();
    let commit = decode("commit", &req.commit)?;
    let info = decode("group_info", &req.group_info)?;
    let epoch = check_mls(project, &commit, Kind::Commit)?;
    let next = check_group_info(project, &info)?;
    if Some(next) != epoch.checked_add(1) {
        return Err(AppError::invalid(
            "the GroupInfo must be for the epoch after the commit",
        ));
    }
    let welcome = match &req.welcome {
        Some(w) => Some(decode("welcome", w)?),
        None => None,
    };
    if req.welcome_to.len() > DEVICES_PER_REQUEST_MAX {
        return Err(AppError::invalid("too many devices"));
    }
    let mut welcome_to = Vec::new();
    if welcome.is_some() {
        let allowed = allowed_devices(&state, access.id()).await?;
        for d in &req.welcome_to {
            let key = decode("welcome_to", d)?;
            if !allowed.iter().any(|(_, k)| *k == key) {
                return Err(AppError::invalid(
                    "a Welcome is addressed to a device that cannot read this project",
                ));
            }
            welcome_to.push(key);
        }
    }
    let seq = state
        .store
        .mls_commit(
            project,
            epoch,
            &commit,
            &info,
            welcome.as_deref().map(|w| (w, welcome_to.as_slice())),
            auth.user.id,
            now_ms(),
        )
        .await?;
    announce(&state, access.id()).await?;
    Ok((
        StatusCode::CREATED,
        Json(CommitResponse { seq, epoch: next }),
    ))
}

/// Project keys are at most a few thousand 32-byte keys.
const KEYS_BYTES_MAX: usize = 512 * 1024;

/// `POST /projects/{id}/mls/keys`: project keys for the group's devices, as an MLS application
/// message of the current epoch. The server cannot read them; it checks they belong to this
/// project's group and epoch, and keeps them in the log.
pub async fn share_keys(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
    Json(req): Json<ShareKeysRequest>,
) -> AppResult<(StatusCode, Json<CommitResponse>)> {
    let access = project_access(&state, ProjectId(id), auth.user.id).await?;
    check_write(&state, auth.user.id)?;
    if req.data.len() > KEYS_BYTES_MAX.div_ceil(3) * 4 {
        return Err(AppError::invalid("too many keys in one message"));
    }
    let data = decode("data", &req.data)?;
    let project = access.id();
    let epoch = check_mls(project, &data, Kind::Application)?;
    let seq = state
        .store
        .mls_share_keys(project, epoch, &data, auth.user.id, now_ms())
        .await?;
    announce(&state, project).await?;
    Ok((StatusCode::CREATED, Json(CommitResponse { seq, epoch })))
}
