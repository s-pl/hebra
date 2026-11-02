//! Small formatting and parsing helpers for human-facing output.

use anyhow::{Result, bail};

/// "3m ago", "2h ago", "5d ago" from an RFC 3339 timestamp.
pub fn ago(timestamp: &str) -> String {
    let Ok(then) = timestamp.parse::<jiff::Timestamp>() else {
        return String::new();
    };
    let secs = jiff::Timestamp::now().as_second() - then.as_second();
    if secs < 0 {
        return "just now".to_owned();
    }
    format!("{} ago", duration(secs))
}

/// Compact duration: `45s`, `12m`, `3h`, `2d`.
pub fn duration(secs: i64) -> String {
    let secs = secs.max(0);
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m", secs / 60),
        3600..86_400 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86_400),
    }
}

/// Parses `90s`, `30m`, `2h`, `1d`; a bare number means minutes.
pub fn parse_ttl(s: &str) -> Result<i64> {
    let s = s.trim();
    let (number, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
    let Ok(n) = number.parse::<i64>() else {
        bail!("invalid duration `{s}` (use for example 90s, 30m, 2h)");
    };
    let factor = match unit {
        "s" => 1,
        "" | "m" => 60,
        "h" => 3600,
        "d" => 86_400,
        _ => bail!("invalid duration unit in `{s}` (use s, m, h or d)"),
    };
    if n <= 0 {
        bail!("duration must be positive");
    }
    Ok(n * factor)
}

/// Colors a unified diff: green additions, red removals, cyan positions.
pub fn color_diff(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 8);
    for line in text.split_inclusive('\n') {
        let code = if line.starts_with("+++") || line.starts_with("---") {
            "1"
        } else if line.starts_with('+') {
            "32"
        } else if line.starts_with('-') {
            "31"
        } else if line.starts_with("@@") {
            "36"
        } else if line.starts_with("diff --git") {
            "1"
        } else {
            out.push_str(line);
            continue;
        };
        let body = line.strip_suffix('\n').unwrap_or(line);
        out.push_str("\x1b[");
        out.push_str(code);
        out.push('m');
        out.push_str(body);
        out.push_str("\x1b[0m");
        if line.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

/// A date as RFC 3339: a full timestamp, `2026-01-02`, or an age (`3d`, `2w`,
/// `2 weeks ago`).
pub fn parse_date(text: &str) -> Result<String> {
    let text = text.trim();
    if let Ok(ts) = text.parse::<jiff::Timestamp>() {
        return Ok(ts.to_string());
    }
    if let Ok(date) = text.parse::<jiff::civil::Date>() {
        let zoned = date
            .at(0, 0, 0, 0)
            .to_zoned(jiff::tz::TimeZone::UTC)
            .map_err(|e| anyhow::anyhow!("invalid date `{text}`: {e}"))?;
        return Ok(zoned.timestamp().to_string());
    }
    if let Ok(zoned) = jiff::fmt::rfc2822::parse(text) {
        return Ok(zoned.timestamp().to_string());
    }
    let words = text.trim_end_matches(" ago").replace(' ', "");
    let digits: String = words.chars().take_while(char::is_ascii_digit).collect();
    let unit = &words[digits.len()..];
    let Ok(n) = digits.parse::<i64>() else {
        bail!("invalid date `{text}` (use 2026-01-02, or an age like 3d or 2w)");
    };
    let seconds = match unit.trim_end_matches('s') {
        "" | "m" | "min" | "minute" => 60,
        "h" | "hour" => 3600,
        "d" | "day" => 86_400,
        "w" | "week" => 604_800,
        "mo" | "month" => 2_592_000,
        "y" | "year" => 31_536_000,
        _ => bail!("invalid date `{text}`"),
    };
    let then = jiff::Timestamp::now().as_second() - n * seconds;
    Ok(jiff::Timestamp::from_second(then)
        .map_err(|e| anyhow::anyhow!("invalid date `{text}`: {e}"))?
        .to_string())
}

/// Workspace name from a task: lowercase words joined by dashes.
pub fn slug(task: &str) -> String {
    let mut out = String::new();
    for word in task
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
    {
        if out.len() + word.len() > 32 {
            break;
        }
        if !out.is_empty() {
            out.push('-');
        }
        out.push_str(&word);
    }
    if out.is_empty() {
        "workspace".to_owned()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ttls() {
        assert_eq!(parse_ttl("90s").unwrap(), 90);
        assert_eq!(parse_ttl("30m").unwrap(), 1800);
        assert_eq!(parse_ttl("2h").unwrap(), 7200);
        assert_eq!(parse_ttl("5").unwrap(), 300);
        assert!(parse_ttl("x").is_err());
        assert!(parse_ttl("3w").is_err());
    }

    #[test]
    fn slugs_tasks() {
        assert_eq!(slug("Add OAuth login!"), "add-oauth-login");
        assert_eq!(slug("¿?"), "workspace");
    }
}
