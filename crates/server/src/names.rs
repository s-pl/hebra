//! Valid names: account handles, repositories, bookmarks, and the `*`
//! patterns used for protected bookmarks.

/// Handles that would collide with routes or confuse people.
const RESERVED: &[&str] = &[
    "about",
    "admin",
    "api",
    "app",
    "billing",
    "blog",
    "dashboard",
    "docs",
    "explore",
    "help",
    "hub",
    "login",
    "logout",
    "new",
    "orgs",
    "pricing",
    "root",
    "security",
    "settings",
    "signup",
    "status",
    "support",
    "system",
    "hebra",
    "www",
];

/// Account and organization handles: 1–39 of `a-z 0-9 -`, starting with a
/// letter or digit, no `--`, not reserved.
#[must_use]
pub fn valid_handle(handle: &str) -> bool {
    !handle.is_empty()
        && handle.len() <= 39
        && handle
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && !handle.ends_with('-')
        && !handle.contains("--")
        && handle
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !RESERVED.contains(&handle)
}

/// Repository names (the part after `owner/`).
#[must_use]
pub fn valid_repo_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
        && !name.contains("..")
        && !name.ends_with(".git")
}

#[must_use]
pub fn valid_bookmark(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.contains("..")
        && !name.contains("//")
        && !name.starts_with('/')
        && !name.ends_with('/')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c))
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

/// Turns a free-form name into a valid handle (`Ana López` → `ana-lopez`).
#[must_use]
pub fn handle_from(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars().flat_map(char::to_lowercase) {
        let c = match c {
            'á' | 'à' | 'ä' | 'â' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            c => c,
        };
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let mut out: String = out.trim_end_matches('-').chars().take(39).collect();
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "user".to_owned()
    } else {
        out
    }
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
        assert!(valid_handle("ana"));
        assert!(valid_handle("acme-labs"));
        assert!(!valid_handle("Ana"));
        assert!(!valid_handle("-ana"));
        assert!(!valid_handle("api"));
        assert!(!valid_handle("a--b"));
    }

    #[test]
    fn handles_from_names() {
        assert_eq!(handle_from("Ana López"), "ana-lopez");
        assert_eq!(handle_from("  ACME Labs!! "), "acme-labs");
        assert_eq!(handle_from("***"), "user");
    }
}
