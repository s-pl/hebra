//! The parts of `.gitattributes` that change what ends up in a file:
//! `text`, `-text`, `binary`, `eol=crlf`, `eol=lf` and `export-ignore`.
//!
//! Text files are stored with LF. A path marked `-text` or `binary` is kept
//! byte for byte, and one marked `eol=crlf` is written with CRLF on disk, as
//! Windows batch files need.

use std::path::Path;

use ignore::gitignore::{Gitignore, GitignoreBuilder};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Attributes {
    /// `Some(false)`: never touch line endings.
    pub text: Option<bool>,
    pub crlf: Option<bool>,
    pub export_ignore: bool,
}

struct Rule {
    matcher: Gitignore,
    set: Vec<String>,
}

/// The rules of a directory's `.gitattributes`.
#[derive(Default)]
pub struct Rules {
    rules: Vec<Rule>,
}

impl std::fmt::Debug for Rules {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Rules({})", self.rules.len())
    }
}

impl Rules {
    /// Reads `.gitattributes` from a workspace root. Lines that make no
    /// sense are skipped, as Git does.
    #[must_use]
    pub fn load(root: &Path) -> Self {
        std::fs::read_to_string(root.join(".gitattributes"))
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut rules = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut words = line.split_whitespace();
            let Some(pattern) = words.next() else {
                continue;
            };
            let mut builder = GitignoreBuilder::new("");
            if builder.add_line(None, pattern).is_err() {
                continue;
            }
            let Ok(matcher) = builder.build() else {
                continue;
            };
            rules.push(Rule {
                matcher,
                set: words.map(str::to_owned).collect(),
            });
        }
        Self { rules }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// What applies to a path. Later lines win, attribute by attribute.
    #[must_use]
    pub fn of(&self, path: &str) -> Attributes {
        let mut out = Attributes::default();
        for rule in &self.rules {
            if !rule
                .matcher
                .matched_path_or_any_parents(path, false)
                .is_ignore()
            {
                continue;
            }
            for attribute in &rule.set {
                match attribute.as_str() {
                    "text" => out.text = Some(true),
                    "-text" | "binary" => out.text = Some(false),
                    "eol=crlf" => {
                        out.text = out.text.or(Some(true));
                        out.crlf = Some(true);
                    }
                    "eol=lf" => {
                        out.text = out.text.or(Some(true));
                        out.crlf = Some(false);
                    }
                    "export-ignore" => out.export_ignore = true,
                    _ => {}
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn later_lines_win_and_patterns_match_like_git() {
        let rules = Rules::parse(
            "# comment\n* text=auto\n*.bat eol=crlf\n*.png binary\nvendor/** -text\ndocs export-ignore\n*.txt text\nkeep.txt -text\n",
        );
        assert_eq!(rules.of("run.bat").crlf, Some(true));
        assert_eq!(rules.of("sub/run.bat").crlf, Some(true));
        assert_eq!(rules.of("a.png").text, Some(false));
        assert_eq!(rules.of("vendor/x/y.c").text, Some(false));
        assert!(rules.of("docs/guide.md").export_ignore);
        assert_eq!(rules.of("a.txt").text, Some(true));
        assert_eq!(rules.of("keep.txt").text, Some(false));
        assert_eq!(rules.of("main.rs"), Attributes::default());
    }
}
