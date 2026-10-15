//! Who may do what: tokens, permission patterns and valid names.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct TokenInfo {
    pub name: String,
    pub kind: String,
    pub on_behalf_of: Option<String>,
    pub allow: Vec<String>,
    pub admin: bool,
}

impl TokenInfo {
    #[must_use]
    pub fn display(&self) -> String {
        match (&self.kind[..], &self.on_behalf_of) {
            ("agent", Some(p)) => format!("{} (agent for {p})", self.name),
            ("agent", None) => format!("{} (agent)", self.name),
            _ => self.name.clone(),
        }
    }
}

/// Tokens carry 192 random bits, so a fast hash is enough to store them.
pub(crate) fn hash_token(token: &str) -> String {
    blake3::hash(token.as_bytes()).to_hex().to_string()
}

/// `*` matches any run of characters; everything else is literal.
#[must_use]
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !text.starts_with(first) || !text.ends_with(last) || text.len() < first.len() + last.len() {
        return false;
    }
    let mut rest = &text[first.len()..text.len() - last.len()];
    for part in &parts[1..parts.len() - 1] {
        match rest.find(part) {
            Some(i) => rest = &rest[i + part.len()..],
            None => return false,
        }
    }
    true
}

#[must_use]
pub fn valid_repo_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
        && !name.contains("..")
}

pub(crate) fn valid_bookmark(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.contains("..")
        && !name.starts_with('/')
        && !name.ends_with('/')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs() {
        assert!(glob_match("*", "main"));
        assert!(glob_match("agents/*", "agents/claude/oauth"));
        assert!(!glob_match("agents/*", "main"));
        assert!(glob_match("release-*-final", "release-1.2-final"));
        assert!(!glob_match("main", "mainx"));
    }

    #[test]
    fn names() {
        assert!(valid_repo_name("acme-app"));
        assert!(!valid_repo_name("../etc"));
        assert!(!valid_repo_name("a..b"));
        assert!(valid_bookmark("agents/claude/oauth"));
        assert!(!valid_bookmark("a/../b"));
    }
}
