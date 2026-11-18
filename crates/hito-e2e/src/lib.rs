//! Hito's end-to-end encryption.
//!
//! Every project is one MLS group (Messaging Layer Security, RFC 9420), run by
//! [OpenMLS](https://github.com/openmls/openmls). Each signed-in client is one MLS member, a
//! *device*, with its own signing key. The group carries no task text itself: it hands out
//! *project keys*, random 256-bit keys that the members then use to encrypt task titles,
//! descriptions, labels, checklists, comments and milestone text with XChaCha20-Poly1305
//! ([`Device::seal`], [`Device::open`]). The server stores and relays only ciphertext, plus the
//! metadata it needs to sort and count (status, priority, dates, assignee, ranks).
//!
//! Keys travel only inside the MLS group, so only its devices get them. A device that joins
//! later receives every key of the project from a member that is online ([`Device::share_keys`]
//! after [`Outcome::Changed`]); when someone is removed, the remover starts a new key
//! ([`Device::rotate`]) so that nothing written afterwards opens with what they had.
//!
//! Nothing here is home-made cryptography: MLS through OpenMLS and AEAD through the
//! `chacha20poly1305` crate, with standard parameters. This crate does no I/O.
//!
//! What the server does decide is who is in a project, so a dishonest server could add a device
//! of its own. Every client shows the devices in a project and their fingerprints, which is how
//! people catch that.

use std::collections::{BTreeMap, BTreeSet};

use chacha20poly1305::aead::{Aead, KeyInit, Payload as AeadPayload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

use base64::Engine as _;
use hito_core::{ProjectId, UserId};
use openmls::messages::group_info::VerifiableGroupInfo;
use openmls::prelude::tls_codec::{Deserialize as _, Serialize as _};
use openmls::prelude::*;
use openmls_basic_credential::SignatureKeyPair;
use openmls_rust_crypto::{MemoryStorage, RustCrypto};
use openmls_traits::OpenMlsProvider;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// X25519 key agreement, AES-128-GCM, SHA-256 and Ed25519 signatures: MLS's mandatory suite.
pub const CIPHERSUITE: Ciphersuite = Ciphersuite::MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519;

/// The version of [`Device::to_bytes`]'s format.
const FORMAT_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("this device's saved keys are damaged or from another version")]
    Corrupt,
    #[error("this device has no keys for that project yet")]
    NoGroup,
    #[error("the project has a commit waiting for the server's answer")]
    Pending,
    #[error("an MLS message is malformed")]
    Malformed,
    #[error("the message belongs to another project")]
    WrongGroup,
    #[error("this device does not have the key that text was written with")]
    NoKey,
    #[error("the text was altered or belongs elsewhere")]
    Tampered,
    #[error("encryption failed: {0}")]
    Mls(String),
}

pub type Result<T> = std::result::Result<T, Error>;

fn mls<E: std::fmt::Debug>(e: E) -> Error {
    Error::Mls(format!("{e:?}"))
}

/// The MLS group id of a project. The server checks it on everything it stores.
pub fn group_id(project: ProjectId) -> Vec<u8> {
    format!("hito:project:{}", project.0).into_bytes()
}

/// What an MLS message says about itself without being decrypted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub group_id: Vec<u8>,
    pub epoch: u64,
    pub kind: Kind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Application,
    Commit,
    Proposal,
}

/// Read the plaintext header of an MLS handshake or application message.
pub fn inspect(bytes: &[u8]) -> Result<Header> {
    let message = MlsMessageIn::tls_deserialize_exact(bytes).map_err(|_| Error::Malformed)?;
    let message = message
        .try_into_protocol_message()
        .map_err(|_| Error::Malformed)?;
    let kind = match message.content_type() {
        ContentType::Application => Kind::Application,
        ContentType::Commit => Kind::Commit,
        ContentType::Proposal => Kind::Proposal,
    };
    Ok(Header {
        group_id: message.group_id().as_slice().to_vec(),
        epoch: message.epoch().as_u64(),
        kind,
    })
}

/// Read the group id and epoch of a GroupInfo, without verifying its signature.
pub fn inspect_group_info(bytes: &[u8]) -> Result<(Vec<u8>, u64)> {
    let info = group_info(bytes)?;
    Ok((info.group_id().as_slice().to_vec(), info.epoch().as_u64()))
}

fn group_info(bytes: &[u8]) -> Result<VerifiableGroupInfo> {
    match MlsMessageIn::tls_deserialize_exact(bytes)
        .map_err(|_| Error::Malformed)?
        .extract()
    {
        MlsMessageBodyIn::GroupInfo(info) => Ok(info),
        _ => Err(Error::Malformed),
    }
}

/// The person and device a key package belongs to, after checking its signature.
pub fn inspect_key_package(bytes: &[u8]) -> Result<Leaf> {
    let kp = KeyPackageIn::tls_deserialize_exact(bytes).map_err(|_| Error::Malformed)?;
    let kp = kp
        .validate(&RustCrypto::default(), ProtocolVersion::Mls10)
        .map_err(|_| Error::Malformed)?;
    if kp.ciphersuite() != CIPHERSUITE {
        return Err(Error::Malformed);
    }
    let node = kp.leaf_node();
    Ok(Leaf {
        user: user_of(node.credential()).ok_or(Error::Malformed)?,
        public_key: node.signature_key().as_slice().to_vec(),
    })
}

/// A short, readable digest of a device's signing key, for people to compare.
pub fn fingerprint(public_key: &[u8]) -> String {
    let digest = Sha256::digest(public_key);
    let hex: Vec<String> = digest.iter().take(12).map(|b| format!("{b:02X}")).collect();
    hex.chunks(2)
        .map(|pair| pair.concat())
        .collect::<Vec<_>>()
        .join(" ")
}

fn identity(user: UserId) -> Vec<u8> {
    format!("hito:user:{}", user.0).into_bytes()
}

fn user_of(credential: &Credential) -> Option<UserId> {
    let basic = BasicCredential::try_from(credential.clone()).ok()?;
    let text = std::str::from_utf8(basic.identity()).ok()?;
    text.strip_prefix("hito:user:")?.parse().ok().map(UserId)
}

/// A member of a project's group: one device of one person.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Leaf {
    pub user: UserId,
    /// The device's signing key, as the server lists it.
    pub public_key: Vec<u8>,
}

impl Leaf {
    pub fn fingerprint(&self) -> String {
        fingerprint(&self.public_key)
    }

    fn from_member(member: &Member) -> Option<Leaf> {
        Some(Leaf {
            user: user_of(&member.credential)?,
            public_key: member.signature_key.clone(),
        })
    }
}

/// One entry of a project's encrypted log, as the server hands it out.
#[derive(Clone, Debug)]
pub struct Entry {
    pub seq: i64,
    pub author: UserId,
    pub data: Vec<u8>,
}

/// What processing an [`Entry`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Project keys arrived; `new` of them were not known before.
    Keys { new: usize },
    /// The group changed: devices joined or left. A member that sees devices join should
    /// [`Device::share_keys`], so they can read what was written before them.
    Changed { joined: Vec<Leaf>, left: Vec<Leaf> },
    /// This device was removed from the project; it must join again to read on.
    Removed,
    /// Nothing to do: this device's own entry.
    Skipped,
    /// The entry could not be decrypted or did not match its metadata.
    Unreadable,
}

/// The most keys one share may carry: one per rotation, so this is far beyond real use.
const KEYS_PER_SHARE_MAX: usize = 4096;

/// The contents of an application message: project keys, by id.
#[derive(Serialize, Deserialize)]
struct KeyShare {
    v: u32,
    keys: Vec<(u32, String)>,
}

/// Text sealed with a project key, as the server stores it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sealed {
    pub key_id: u32,
    /// A 24-byte nonce followed by the ciphertext and its 16-byte tag.
    pub data: Vec<u8>,
}

/// What a sealed text is, bound into its encryption so that a server cannot move a comment
/// into a task title, or one project's text into another.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Task,
    Comment,
    Milestone,
    Project,
    Activity,
}

impl Field {
    fn as_str(self) -> &'static str {
        match self {
            Field::Task => "task",
            Field::Comment => "comment",
            Field::Milestone => "milestone",
            Field::Project => "project",
            Field::Activity => "activity",
        }
    }
}

fn aad(project: ProjectId, field: Field, key_id: u32) -> Vec<u8> {
    format!("hito:v1:{}:{}:{}", project.0, field.as_str(), key_id).into_bytes()
}

/// A handshake waiting for the server: send `commit` and `group_info`, then call
/// [`Device::confirm`] with the entry's sequence number, or [`Device::abort`] if refused.
#[derive(Clone, Debug)]
pub struct Pending {
    pub commit: Vec<u8>,
    pub group_info: Vec<u8>,
    pub epoch: u64,
}

#[derive(Default, Serialize, Deserialize)]
struct ProjectState {
    /// The last log entry this device has processed.
    last_seq: i64,
    /// Entries this device wrote itself; MLS cannot decrypt its own messages.
    own: BTreeSet<i64>,
    /// A commit sent but not yet answered.
    #[serde(default)]
    pending: bool,
    /// An external join sent but not yet answered.
    #[serde(default)]
    joining: bool,
}

#[derive(Default, Serialize, Deserialize)]
struct State {
    projects: BTreeMap<i64, ProjectState>,
    /// Every project key this device has, by project and key id, base64.
    keys: BTreeMap<i64, BTreeMap<u32, String>>,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    version: u32,
    user: i64,
    public_key: String,
    storage: Vec<(String, String)>,
    state: State,
}

#[derive(Default)]
struct Provider {
    crypto: RustCrypto,
    storage: MemoryStorage,
}

impl OpenMlsProvider for Provider {
    type CryptoProvider = RustCrypto;
    type RandProvider = RustCrypto;
    type StorageProvider = MemoryStorage;

    fn storage(&self) -> &MemoryStorage {
        &self.storage
    }
    fn crypto(&self) -> &RustCrypto {
        &self.crypto
    }
    fn rand(&self) -> &RustCrypto {
        &self.crypto
    }
}

/// One signed-in client's keys and groups.
pub struct Device {
    user: UserId,
    provider: Provider,
    signer: SignatureKeyPair,
    state: State,
}

impl std::fmt::Debug for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Device")
            .field("user", &self.user)
            .field("fingerprint", &self.fingerprint())
            .finish_non_exhaustive()
    }
}

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

impl Device {
    /// A new device for `user`, with a fresh signing key.
    pub fn new(user: UserId) -> Result<Device> {
        let provider = Provider::default();
        let signer = SignatureKeyPair::new(CIPHERSUITE.signature_algorithm()).map_err(mls)?;
        signer.store(provider.storage()).map_err(mls)?;
        Ok(Device {
            user,
            provider,
            signer,
            state: State::default(),
        })
    }

    /// Everything this device knows, to store on disk (owner-only: it holds private keys).
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let values = self
            .provider
            .storage
            .values
            .read()
            .map_err(|_| Error::Corrupt)?;
        let storage = values
            .iter()
            .map(|(k, v)| (b64().encode(k), b64().encode(v)))
            .collect();
        drop(values);
        let saved = SavedRef {
            version: FORMAT_VERSION,
            user: self.user.0,
            public_key: b64().encode(self.signer.public()),
            storage,
            state: &self.state,
        };
        serde_json::to_vec(&saved).map_err(|_| Error::Corrupt)
    }

    /// A device saved with [`Device::to_bytes`].
    pub fn from_bytes(bytes: &[u8]) -> Result<Device> {
        let saved: Saved = serde_json::from_slice(bytes).map_err(|_| Error::Corrupt)?;
        if saved.version != FORMAT_VERSION {
            return Err(Error::Corrupt);
        }
        let provider = Provider::default();
        {
            let mut values = provider
                .storage
                .values
                .write()
                .map_err(|_| Error::Corrupt)?;
            for (k, v) in saved.storage {
                let k = b64().decode(k).map_err(|_| Error::Corrupt)?;
                let v = b64().decode(v).map_err(|_| Error::Corrupt)?;
                values.insert(k, v);
            }
        }
        let public = b64().decode(saved.public_key).map_err(|_| Error::Corrupt)?;
        let signer = SignatureKeyPair::read(
            provider.storage(),
            &public,
            CIPHERSUITE.signature_algorithm(),
        )
        .ok_or(Error::Corrupt)?;
        Ok(Device {
            user: UserId(saved.user),
            provider,
            signer,
            state: saved.state,
        })
    }

    pub fn user(&self) -> UserId {
        self.user
    }

    /// This device's signing key, which the server lists beside the session.
    pub fn public_key(&self) -> Vec<u8> {
        self.signer.public().to_vec()
    }

    pub fn fingerprint(&self) -> String {
        fingerprint(self.signer.public())
    }

    fn credential(&self) -> CredentialWithKey {
        CredentialWithKey {
            credential: BasicCredential::new(identity(self.user)).into(),
            signature_key: self.signer.public().into(),
        }
    }

    fn join_config() -> MlsGroupJoinConfig {
        MlsGroupJoinConfig::builder()
            .use_ratchet_tree_extension(true)
            .wire_format_policy(PURE_CIPHERTEXT_WIRE_FORMAT_POLICY)
            // Keys of past epochs are dropped at once: a removed device's old keys open nothing.
            .max_past_epochs(0)
            .build()
    }

    fn load(&self, project: ProjectId) -> Result<Option<MlsGroup>> {
        MlsGroup::load(
            self.provider.storage(),
            &GroupId::from_slice(&group_id(project)),
        )
        .map_err(mls)
    }

    fn group(&self, project: ProjectId) -> Result<MlsGroup> {
        self.load(project)?.ok_or(Error::NoGroup)
    }

    fn project(&mut self, project: ProjectId) -> &mut ProjectState {
        self.state.projects.entry(project.0).or_default()
    }

    /// Whether this device can read and write `project` now.
    pub fn is_member(&self, project: ProjectId) -> bool {
        matches!(self.load(project), Ok(Some(g)) if g.is_active())
    }

    /// The group's epoch, if this device is in it.
    pub fn epoch(&self, project: ProjectId) -> Option<u64> {
        match self.load(project) {
            Ok(Some(g)) if g.is_active() => Some(g.epoch().as_u64()),
            _ => None,
        }
    }

    /// The last log entry processed for `project`.
    pub fn last_seq(&self, project: ProjectId) -> i64 {
        self.state
            .projects
            .get(&project.0)
            .map_or(0, |c| c.last_seq)
    }

    /// The devices in `project`'s group.
    pub fn members(&self, project: ProjectId) -> Vec<Leaf> {
        match self.load(project) {
            Ok(Some(g)) if g.is_active() => {
                g.members().filter_map(|m| Leaf::from_member(&m)).collect()
            }
            _ => Vec::new(),
        }
    }

    /// Start `project`'s group with this device alone. Returns its GroupInfo for the server;
    /// call [`Device::abort`] if the server already has a group.
    pub fn create(&mut self, project: ProjectId, from_seq: i64) -> Result<Vec<u8>> {
        self.forget(project)?;
        let config = MlsGroupCreateConfig::builder()
            .ciphersuite(CIPHERSUITE)
            .use_ratchet_tree_extension(true)
            .wire_format_policy(PURE_CIPHERTEXT_WIRE_FORMAT_POLICY)
            .max_past_epochs(0)
            .build();
        let group = MlsGroup::new_with_group_id(
            &self.provider,
            &self.signer,
            &config,
            GroupId::from_slice(&group_id(project)),
            self.credential(),
        )
        .map_err(mls)?;
        let info = group
            .export_group_info(self.provider.crypto(), &self.signer, true)
            .map_err(mls)?;
        let state = self.project(project);
        state.last_seq = from_seq;
        state.joining = true;
        info.tls_serialize_detached().map_err(mls)
    }

    /// Join `project`'s group from the server's latest GroupInfo, by an external commit.
    /// Messages sent before this device joined stay unreadable to it.
    pub fn join(
        &mut self,
        project: ProjectId,
        group_info_bytes: &[u8],
        from_seq: i64,
    ) -> Result<Pending> {
        let info = group_info(group_info_bytes)?;
        if info.group_id().as_slice() != group_id(project).as_slice() {
            return Err(Error::WrongGroup);
        }
        self.forget(project)?;
        let (group, bundle) = MlsGroup::external_commit_builder()
            .with_config(Self::join_config())
            .build_group(&self.provider, info, self.credential())
            .map_err(mls)?
            .load_psks(self.provider.storage())
            .map_err(mls)?
            .build(
                self.provider.rand(),
                self.provider.crypto(),
                &self.signer,
                |_| true,
            )
            .map_err(mls)?
            .finalize(&self.provider)
            .map_err(mls)?;
        let (commit, _, info) = bundle.into_contents();
        let info = info.ok_or(Error::Mls("no GroupInfo after joining".into()))?;
        let pending = Pending {
            commit: commit.tls_serialize_detached().map_err(mls)?,
            group_info: MlsMessageOut::from(info)
                .tls_serialize_detached()
                .map_err(mls)?,
            epoch: group.epoch().as_u64().saturating_sub(1),
        };
        let state = self.project(project);
        state.last_seq = from_seq;
        state.joining = true;
        state.pending = false;
        Ok(pending)
    }

    /// `count` fresh key packages: what others need to add this device to a project while it
    /// is offline. Each is used once; the private halves stay in this device's storage.
    pub fn key_packages(&mut self, count: usize) -> Result<Vec<Vec<u8>>> {
        (0..count)
            .map(|_| {
                let bundle = KeyPackage::builder()
                    .build(CIPHERSUITE, &self.provider, &self.signer, self.credential())
                    .map_err(mls)?;
                bundle.key_package().tls_serialize_detached().map_err(mls)
            })
            .collect()
    }

    /// Add other people's devices from their key packages. Returns the commit for the server
    /// and the Welcome that lets the new devices in; `None` if no key package was usable.
    /// Each key package must belong to the user and signing key the server listed it under.
    pub fn add(
        &mut self,
        project: ProjectId,
        key_packages: &[(Leaf, Vec<u8>)],
    ) -> Result<Option<(Pending, Vec<u8>)>> {
        let mut group = self.group(project)?;
        if group.pending_commit().is_some() {
            return Err(Error::Pending);
        }
        let present: Vec<Leaf> = group
            .members()
            .filter_map(|m| Leaf::from_member(&m))
            .collect();
        let mut valid: Vec<KeyPackage> = Vec::new();
        for (leaf, bytes) in key_packages {
            let Ok(kp) = KeyPackageIn::tls_deserialize_exact(bytes.as_slice()) else {
                continue;
            };
            let Ok(kp) = kp.validate(self.provider.crypto(), ProtocolVersion::Mls10) else {
                continue;
            };
            let node = kp.leaf_node();
            let claimed = Leaf {
                user: match user_of(node.credential()) {
                    Some(u) => u,
                    None => continue,
                },
                public_key: node.signature_key().as_slice().to_vec(),
            };
            if claimed != *leaf || present.contains(&claimed) || kp.ciphersuite() != CIPHERSUITE {
                continue;
            }
            if valid
                .iter()
                .any(|v| v.leaf_node().signature_key() == node.signature_key())
            {
                continue;
            }
            valid.push(kp);
        }
        if valid.is_empty() {
            return Ok(None);
        }
        let epoch = group.epoch().as_u64();
        let (commit, welcome, info) = group
            .add_members(&self.provider, &self.signer, &valid)
            .map_err(mls)?;
        let info = info.ok_or(Error::Mls("no GroupInfo after adding".into()))?;
        self.project(project).pending = true;
        Ok(Some((
            Pending {
                commit: commit.tls_serialize_detached().map_err(mls)?,
                group_info: MlsMessageOut::from(info)
                    .tls_serialize_detached()
                    .map_err(mls)?,
                epoch,
            },
            welcome.tls_serialize_detached().map_err(mls)?,
        )))
    }

    /// Enter `project`'s group through a Welcome that another member sent with the commit at
    /// log entry `seq`. Everything after that entry is readable.
    pub fn welcome(&mut self, project: ProjectId, bytes: &[u8], seq: i64) -> Result<()> {
        let MlsMessageBodyIn::Welcome(welcome) = MlsMessageIn::tls_deserialize_exact(bytes)
            .map_err(|_| Error::Malformed)?
            .extract()
        else {
            return Err(Error::Malformed);
        };
        self.forget(project)?;
        let group =
            StagedWelcome::new_from_welcome(&self.provider, &Self::join_config(), welcome, None)
                .map_err(mls)?
                .into_group(&self.provider)
                .map_err(mls)?;
        if group.group_id().as_slice() != group_id(project).as_slice() {
            let mut group = group;
            group.delete(self.provider.storage()).map_err(mls)?;
            return Err(Error::WrongGroup);
        }
        self.project(project).last_seq = seq;
        Ok(())
    }

    /// Remove every device for which `keep` answers false (people who left the project, devices
    /// that signed out). `None` when there is nobody to remove.
    pub fn remove(
        &mut self,
        project: ProjectId,
        keep: impl Fn(&Leaf) -> bool,
    ) -> Result<Option<Pending>> {
        let mut group = self.group(project)?;
        if group.pending_commit().is_some() {
            return Err(Error::Pending);
        }
        let own = group.own_leaf_index();
        let gone: Vec<LeafNodeIndex> = group
            .members()
            .filter(|m| m.index != own)
            .filter(|m| Leaf::from_member(m).is_none_or(|leaf| !keep(&leaf)))
            .map(|m| m.index)
            .collect();
        if gone.is_empty() {
            return Ok(None);
        }
        let epoch = group.epoch().as_u64();
        let (commit, _, info) = group
            .remove_members(&self.provider, &self.signer, &gone)
            .map_err(mls)?;
        let info = info.ok_or(Error::Mls("no GroupInfo after removing".into()))?;
        self.project(project).pending = true;
        Ok(Some(Pending {
            commit: commit.tls_serialize_detached().map_err(mls)?,
            group_info: MlsMessageOut::from(info)
                .tls_serialize_detached()
                .map_err(mls)?,
            epoch,
        }))
    }

    /// The server accepted this device's commit (or group) as entry `seq`.
    pub fn confirm(&mut self, project: ProjectId, seq: Option<i64>) -> Result<()> {
        let mut group = self.group(project)?;
        if group.pending_commit().is_some() {
            group.merge_pending_commit(&self.provider).map_err(mls)?;
        }
        let state = self.project(project);
        state.pending = false;
        state.joining = false;
        if let Some(seq) = seq {
            state.own.insert(seq);
        }
        Ok(())
    }

    /// The server refused this device's commit: drop it (and, for a join, the whole group).
    pub fn abort(&mut self, project: ProjectId) -> Result<()> {
        let joining = self.project(project).joining;
        if joining {
            return self.forget(project);
        }
        if let Some(mut group) = self.load(project)? {
            group
                .clear_pending_commit(self.provider.storage())
                .map_err(mls)?;
        }
        self.project(project).pending = false;
        Ok(())
    }

    /// Drop `project`'s group and position (not its project keys).
    pub fn forget(&mut self, project: ProjectId) -> Result<()> {
        if let Some(mut group) = self.load(project)? {
            group.delete(self.provider.storage()).map_err(mls)?;
        }
        self.state.projects.remove(&project.0);
        Ok(())
    }

    /// The newest key of `project` this device has, if any.
    pub fn current_key(&self, project: ProjectId) -> Option<u32> {
        self.state
            .keys
            .get(&project.0)
            .and_then(|k| k.keys().next_back().copied())
    }

    /// Whether this device can write in `project`: it has a key.
    pub fn has_keys(&self, project: ProjectId) -> bool {
        self.current_key(project).is_some()
    }

    fn key(&self, project: ProjectId, key_id: u32) -> Option<[u8; 32]> {
        let encoded = self.state.keys.get(&project.0)?.get(&key_id)?;
        let bytes = b64().decode(encoded).ok()?;
        bytes.try_into().ok()
    }

    /// Start a new key for `project`, newer than every key this device knows, and return the
    /// application message that hands every key of the project to the group. Used when a
    /// project's group is created and after someone is removed.
    pub fn rotate(&mut self, project: ProjectId) -> Result<(Vec<u8>, u64)> {
        let next = match self.current_key(project) {
            Some(id) => id.checked_add(1).ok_or(Error::Malformed)?,
            None => 1,
        };
        let key = self.provider.rand().random_array::<32>().map_err(mls)?;
        self.state
            .keys
            .entry(project.0)
            .or_default()
            .insert(next, b64().encode(key));
        self.share_keys(project)
    }

    /// The application message that hands every key of `project` this device has to the
    /// group's current devices. Send it when devices join, so they can read earlier text.
    pub fn share_keys(&mut self, project: ProjectId) -> Result<(Vec<u8>, u64)> {
        let mut group = self.group(project)?;
        if group.pending_commit().is_some() {
            return Err(Error::Pending);
        }
        let keys: Vec<(u32, String)> = self
            .state
            .keys
            .get(&project.0)
            .map(|k| k.iter().map(|(id, key)| (*id, key.clone())).collect())
            .unwrap_or_default();
        if keys.is_empty() {
            return Err(Error::NoKey);
        }
        let payload = serde_json::to_vec(&KeyShare { v: 1, keys }).map_err(|_| Error::Malformed)?;
        let message = group
            .create_message(&self.provider, &self.signer, &payload)
            .map_err(mls)?;
        Ok((
            message.tls_serialize_detached().map_err(mls)?,
            group.epoch().as_u64(),
        ))
    }

    /// Record an application message this device sent, by its log entry.
    pub fn sent(&mut self, project: ProjectId, seq: i64) {
        self.project(project).own.insert(seq);
    }

    /// Encrypt `text` with `project`'s newest key.
    pub fn seal(&self, project: ProjectId, field: Field, text: &str) -> Result<Sealed> {
        let key_id = self.current_key(project).ok_or(Error::NoKey)?;
        let key = self.key(project, key_id).ok_or(Error::NoKey)?;
        let cipher = XChaCha20Poly1305::new(&key.into());
        let nonce_bytes = self.provider.rand().random_array::<24>().map_err(mls)?;
        let nonce = XNonce::from(nonce_bytes);
        let aad = aad(project, field, key_id);
        let ciphertext = cipher
            .encrypt(
                &nonce,
                AeadPayload {
                    msg: text.as_bytes(),
                    aad: &aad,
                },
            )
            .map_err(|_| Error::Mls("encryption failed".into()))?;
        let mut data = nonce_bytes.to_vec();
        data.extend_from_slice(&ciphertext);
        Ok(Sealed { key_id, data })
    }

    /// Decrypt text sealed in `project` for `field`.
    pub fn open(&self, project: ProjectId, field: Field, sealed: &Sealed) -> Result<String> {
        let key = self.key(project, sealed.key_id).ok_or(Error::NoKey)?;
        let (nonce, ciphertext) = sealed.data.split_at_checked(24).ok_or(Error::Tampered)?;
        let nonce = XNonce::from_slice(nonce);
        let cipher = XChaCha20Poly1305::new(&key.into());
        let aad = aad(project, field, sealed.key_id);
        let plain = cipher
            .decrypt(
                nonce,
                AeadPayload {
                    msg: ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| Error::Tampered)?;
        String::from_utf8(plain).map_err(|_| Error::Tampered)
    }

    /// Process the next entry of `project`'s log. Entries must come in order.
    pub fn process(&mut self, project: ProjectId, entry: &Entry) -> Result<Outcome> {
        let state = self.project(project);
        if entry.seq <= state.last_seq {
            return Ok(Outcome::Skipped);
        }
        state.last_seq = entry.seq;
        if state.own.remove(&entry.seq) || entry.data.is_empty() {
            return Ok(Outcome::Skipped);
        }
        let Some(mut group) = self.load(project)? else {
            return Ok(Outcome::Unreadable);
        };
        if !group.is_active() {
            return Ok(Outcome::Unreadable);
        }
        let Ok(message) = MlsMessageIn::tls_deserialize_exact(entry.data.as_slice()) else {
            return Ok(Outcome::Unreadable);
        };
        let Ok(message) = message.try_into_protocol_message() else {
            return Ok(Outcome::Unreadable);
        };
        if message.group_id().as_slice() != group_id(project).as_slice() {
            return Ok(Outcome::Unreadable);
        }
        let before: Vec<Member> = group.members().collect();
        let processed = match group.process_message(&self.provider, message) {
            Ok(p) => p,
            // A message from an epoch this device never had keys for, or tampered with.
            Err(_) => return Ok(Outcome::Unreadable),
        };
        let sender = user_of(processed.credential());
        let new_member = matches!(processed.sender(), Sender::NewMemberCommit);
        match processed.into_content() {
            ProcessedMessageContent::ApplicationMessage(app) => {
                if sender != Some(entry.author) {
                    return Ok(Outcome::Unreadable);
                }
                let Ok(share) = serde_json::from_slice::<KeyShare>(&app.into_bytes()) else {
                    return Ok(Outcome::Unreadable);
                };
                if share.v != 1 || share.keys.len() > KEYS_PER_SHARE_MAX {
                    return Ok(Outcome::Unreadable);
                }
                let mut new = 0;
                for (id, key) in share.keys {
                    // A sender's client is not trusted either: only well-formed 256-bit keys.
                    let valid = b64().decode(&key).is_ok_and(|k| k.len() == 32);
                    if !valid {
                        continue;
                    }
                    let keys = self.state.keys.entry(project.0).or_default();
                    // A key id, once known, never changes: a member cannot swap one out.
                    if let std::collections::btree_map::Entry::Vacant(slot) = keys.entry(id) {
                        slot.insert(key);
                        new += 1;
                    }
                }
                Ok(Outcome::Keys { new })
            }
            ProcessedMessageContent::StagedCommitMessage(staged) => {
                if staged.self_removed() {
                    group
                        .merge_staged_commit(&self.provider, *staged)
                        .map_err(mls)?;
                    group.delete(self.provider.storage()).map_err(mls)?;
                    return Ok(Outcome::Removed);
                }
                let left: Vec<Leaf> = staged
                    .remove_proposals()
                    .filter_map(|p| {
                        let index = p.remove_proposal().removed();
                        before.iter().find(|m| m.index == index)
                    })
                    .filter_map(Leaf::from_member)
                    .collect();
                let mut joined: Vec<Leaf> = staged
                    .add_proposals()
                    .filter_map(|p| {
                        let leaf = p.add_proposal().key_package().leaf_node();
                        Some(Leaf {
                            user: user_of(leaf.credential())?,
                            public_key: leaf.signature_key().as_slice().to_vec(),
                        })
                    })
                    .collect();
                if new_member {
                    if let Some(leaf) = staged.update_path_leaf_node() {
                        if let Some(user) = user_of(leaf.credential()) {
                            joined.push(Leaf {
                                user,
                                public_key: leaf.signature_key().as_slice().to_vec(),
                            });
                        }
                    }
                }
                group
                    .merge_staged_commit(&self.provider, *staged)
                    .map_err(mls)?;
                Ok(Outcome::Changed { joined, left })
            }
            // Proposals are not used by Hito's clients; ignore stray ones.
            _ => Ok(Outcome::Skipped),
        }
    }

    /// Drop a project's keys too, when it is deleted or this person leaves it for good.
    pub fn forget_keys(&mut self, project: ProjectId) {
        self.state.keys.remove(&project.0);
    }
}

#[derive(Serialize)]
struct SavedRef<'a> {
    version: u32,
    user: i64,
    public_key: String,
    storage: Vec<(String, String)>,
    state: &'a State,
}

#[cfg(test)]
mod tests;
