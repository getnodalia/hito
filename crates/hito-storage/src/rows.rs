//! Converting rows into protocol types.

use hito_core::{Priority, Role, Status, UserId};
use hito_protocol::{Sealed, User};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use crate::{Result, StoreError};

pub(crate) fn user(row: &SqliteRow, prefix: &str) -> Result<User> {
    Ok(User {
        id: UserId(row.try_get(format!("{prefix}id").as_str())?),
        username: row.try_get(format!("{prefix}username").as_str())?,
        display_name: row.try_get(format!("{prefix}display_name").as_str())?,
    })
}

/// A user from optional joined columns (an author who may have been deleted).
pub(crate) fn opt_user(row: &SqliteRow, prefix: &str) -> Result<Option<User>> {
    let id: Option<i64> = row.try_get(format!("{prefix}id").as_str())?;
    match id {
        None => Ok(None),
        Some(_) => user(row, prefix).map(Some),
    }
}

pub(crate) fn role(s: &str) -> Result<Role> {
    Role::parse(s).ok_or_else(|| StoreError::Corrupt(format!("unknown role {s:?}")))
}

pub(crate) fn status(s: &str) -> Result<Status> {
    Status::parse(s).ok_or_else(|| StoreError::Corrupt(format!("unknown status {s:?}")))
}

pub(crate) fn priority(s: &str) -> Result<Priority> {
    Priority::parse(s).ok_or_else(|| StoreError::Corrupt(format!("unknown priority {s:?}")))
}

pub(crate) fn count(v: i64) -> u32 {
    u32::try_from(v.max(0)).unwrap_or(u32::MAX)
}

/// Sealed text stored as a key id and base64 columns.
pub(crate) fn sealed(row: &SqliteRow, data: &str, key: &str) -> Result<Sealed> {
    opt_sealed(row, data, key)?.ok_or_else(|| StoreError::Corrupt(format!("missing {data}")))
}

pub(crate) fn opt_sealed(row: &SqliteRow, data: &str, key: &str) -> Result<Option<Sealed>> {
    let text: Option<String> = row.try_get(data)?;
    let id: Option<i64> = row.try_get(key)?;
    match (text, id) {
        (Some(data), Some(id)) => Ok(Some(Sealed {
            key_id: u32::try_from(id)
                .map_err(|_| StoreError::Corrupt(format!("bad key id {id}")))?,
            data,
        })),
        _ => Ok(None),
    }
}
