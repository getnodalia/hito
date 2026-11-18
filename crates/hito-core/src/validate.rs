//! Input validation. Everything that arrives from a client passes through one of these functions
//! before it is stored, and each returns the normalised value that is stored.

use crate::limits::*;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ValidationError {
    #[error("{field} is empty")]
    Empty { field: &'static str },
    #[error("{field} is too long (at most {max} characters)")]
    TooLong { field: &'static str, max: usize },
    #[error("{field} is too short (at least {min} characters)")]
    TooShort { field: &'static str, min: usize },
    #[error("{field} contains characters that are not allowed")]
    BadCharacters { field: &'static str },
}

impl ValidationError {
    pub fn field(&self) -> &'static str {
        match self {
            Self::Empty { field }
            | Self::TooLong { field, .. }
            | Self::TooShort { field, .. }
            | Self::BadCharacters { field } => field,
        }
    }
}

fn chars(s: &str) -> usize {
    s.chars().count()
}

/// Control characters other than newline and tab never belong in user text: they can reorder or
/// hide what other people see (bidirectional overrides are included).
fn is_forbidden_control(c: char) -> bool {
    (c.is_control() && c != '\n' && c != '\t')
        || matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// A single-line name: trimmed, no control characters, no newlines.
fn line(field: &'static str, value: &str, max: usize) -> Result<String, ValidationError> {
    let v = value.trim();
    if v.is_empty() {
        return Err(ValidationError::Empty { field });
    }
    if chars(v) > max {
        return Err(ValidationError::TooLong { field, max });
    }
    if v.chars()
        .any(|c| c == '\n' || c == '\t' || is_forbidden_control(c))
    {
        return Err(ValidationError::BadCharacters { field });
    }
    Ok(v.to_owned())
}

/// Usernames are lower case so `Ana` and `ana` cannot be two people.
pub fn username(value: &str) -> Result<String, ValidationError> {
    let field = "username";
    let v = value.trim().to_lowercase();
    if v.is_empty() {
        return Err(ValidationError::Empty { field });
    }
    if chars(&v) < USERNAME_MIN {
        return Err(ValidationError::TooShort {
            field,
            min: USERNAME_MIN,
        });
    }
    if chars(&v) > USERNAME_MAX {
        return Err(ValidationError::TooLong {
            field,
            max: USERNAME_MAX,
        });
    }
    if !v
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
    {
        return Err(ValidationError::BadCharacters { field });
    }
    Ok(v)
}

pub fn display_name(value: &str) -> Result<String, ValidationError> {
    line("display_name", value, DISPLAY_NAME_MAX)
}

/// Passwords are not trimmed or normalised: what the person typed is what is hashed.
pub fn password(value: &str) -> Result<(), ValidationError> {
    let field = "password";
    let n = chars(value);
    if n < PASSWORD_MIN {
        return Err(ValidationError::TooShort {
            field,
            min: PASSWORD_MIN,
        });
    }
    if n > PASSWORD_MAX {
        return Err(ValidationError::TooLong {
            field,
            max: PASSWORD_MAX,
        });
    }
    Ok(())
}


pub fn project_name(value: &str) -> Result<String, ValidationError> {
    line("project_name", value, PROJECT_NAME_MAX)
}

pub fn milestone_name(value: &str) -> Result<String, ValidationError> {
    line("milestone_name", value, MILESTONE_NAME_MAX)
}

pub fn task_title(value: &str) -> Result<String, ValidationError> {
    line("title", value, TASK_TITLE_MAX)
}

/// A label: one short line, kept as typed (labels are case-sensitive, `UX` is not `ux`).
pub fn label(value: &str) -> Result<String, ValidationError> {
    line("label", value, LABEL_MAX)
}

/// The labels of one task: each valid, no duplicates, at most [`LABELS_PER_TASK`].
pub fn labels(values: &[String]) -> Result<Vec<String>, ValidationError> {
    if values.len() > LABELS_PER_TASK {
        return Err(ValidationError::TooLong {
            field: "labels",
            max: LABELS_PER_TASK,
        });
    }
    let mut out: Vec<String> = Vec::with_capacity(values.len());
    for v in values {
        let v = label(v)?;
        if !out.contains(&v) {
            out.push(v);
        }
    }
    Ok(out)
}

pub fn checklist_item(value: &str) -> Result<String, ValidationError> {
    line("checklist", value, CHECKLIST_ITEM_MAX)
}

/// Multi-line text that may be empty (a description): line endings normalised to `\n`, trailing
/// whitespace trimmed. Leading spaces are kept because they matter inside code blocks.
fn text(field: &'static str, value: &str, max: usize) -> Result<String, ValidationError> {
    let v = value.replace("\r\n", "\n").replace('\r', "\n");
    let v = v.trim_end().trim_start_matches('\n');
    if chars(v) > max {
        return Err(ValidationError::TooLong { field, max });
    }
    if v.chars().any(is_forbidden_control) {
        return Err(ValidationError::BadCharacters { field });
    }
    Ok(v.to_owned())
}

/// A project, milestone or task description. May be empty.
pub fn description(value: &str) -> Result<String, ValidationError> {
    text("description", value, DESCRIPTION_MAX)
}

/// A comment on a task: like a description, but never empty.
pub fn comment_body(value: &str) -> Result<String, ValidationError> {
    let v = text("body", value, COMMENT_MAX)?;
    if v.trim().is_empty() {
        return Err(ValidationError::Empty { field: "body" });
    }
    Ok(v)
}

/// The name an API token is shown under ("Ágora", "Build server").
pub fn token_name(value: &str) -> Result<String, ValidationError> {
    line("name", value, TOKEN_NAME_MAX)
}

pub fn search_query(value: &str) -> Result<String, ValidationError> {
    line("query", value, SEARCH_QUERY_MAX)
}

/// A web address the server will call (an Ágora server): `http://` or `https://`, no spaces.
pub fn server_url(value: &str) -> Result<String, ValidationError> {
    let field = "url";
    let v = value.trim().trim_end_matches('/');
    if v.is_empty() {
        return Err(ValidationError::Empty { field });
    }
    if v.len() > URL_MAX {
        return Err(ValidationError::TooLong {
            field,
            max: URL_MAX,
        });
    }
    let rest = v
        .strip_prefix("https://")
        .or_else(|| v.strip_prefix("http://"));
    match rest {
        Some(host) if !host.is_empty() && !host.chars().any(|c| c.is_whitespace() || c.is_control()) => {
            Ok(v.to_owned())
        }
        _ => Err(ValidationError::BadCharacters { field }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usernames_are_lower_case_ascii() {
        assert_eq!(username(" Ana.Ruiz ").unwrap(), "ana.ruiz");
        assert!(username("an").is_err());
        assert!(username("ana ruiz").is_err());
        assert!(username("añá").is_err());
    }

    #[test]
    fn names_are_single_trimmed_lines() {
        assert_eq!(project_name("  Web 2027 ").unwrap(), "Web 2027");
        assert_eq!(milestone_name("Beta pública").unwrap(), "Beta pública");
        assert!(task_title("a\nb").is_err());
        assert!(task_title("   ").is_err());
        assert!(task_title(&"x".repeat(TASK_TITLE_MAX + 1)).is_err());
    }

    #[test]
    fn descriptions_may_be_empty_but_comments_may_not() {
        assert_eq!(description("").unwrap(), "");
        assert_eq!(description("  code\r\nline  \n").unwrap(), "  code\nline");
        assert!(comment_body(" \n ").is_err());
        assert_eq!(comment_body("Hecho ✅").unwrap(), "Hecho ✅");
    }

    #[test]
    fn text_refuses_hidden_direction_overrides() {
        assert!(description("abc\u{202E}def").is_err());
        assert!(comment_body("abc\u{2066}def").is_err());
    }

    #[test]
    fn labels_are_deduplicated_and_bounded() {
        let l = labels(&["UX".into(), "UX".into(), "backend".into()]).unwrap();
        assert_eq!(l, vec!["UX".to_string(), "backend".to_string()]);
        let many: Vec<String> = (0..=LABELS_PER_TASK).map(|i| format!("l{i}")).collect();
        assert!(labels(&many).is_err());
        assert!(labels(&["".into()]).is_err());
    }

    #[test]
    fn server_urls_need_a_scheme() {
        assert_eq!(
            server_url("https://chat.example.com/").unwrap(),
            "https://chat.example.com"
        );
        assert!(server_url("chat.example.com").is_err());
        assert!(server_url("https://").is_err());
        assert!(server_url("https://a b").is_err());
    }

    #[test]
    fn passwords_are_bounded() {
        assert!(password("short").is_err());
        assert!(password("long enough password").is_ok());
        assert!(password(&"x".repeat(PASSWORD_MAX + 1)).is_err());
    }
}
