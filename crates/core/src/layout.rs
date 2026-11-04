//! Merging text that has no grammar here by its layout: indented blocks
//! (most languages and configuration files), Markdown sections, list items
//! and paragraphs, INI sections.
//!
//! A block starts on a line with no indentation, outside brackets and
//! quotes, that does not close something (`}`, `end`, `fi`…); the lines
//! indented under it, and the comments and blank lines just before it, are
//! its own. Its key is the start of its first line (`fun foo`, `build`,
//! `FROM rust`, `[server]`). Blocks merge like the parts of
//! [`crate::structural`]: when both sides changed one, what is indented
//! inside it is merged the same way. Files whose brackets do not balance,
//! or whose code fences do not close, are left to the line merge.

use crate::structural::{Piece, merge_pieces, merge_text};

/// How many levels a merge goes into nested blocks.
const MAX_NESTING: usize = 6;
/// Keys longer than this are cut.
const MAX_KEY: usize = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Style {
    /// Indented blocks.
    Blocks,
    /// Sections by heading, then paragraphs and list items.
    Markdown,
    /// Paragraphs and list items.
    Paragraphs,
    /// `[section]` headers, then one line per key.
    Sections,
}

fn style(path: &str) -> Style {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    let ext = name.rsplit_once('.').map_or("", |(_, e)| e);
    match ext {
        "md" | "markdown" | "mdx" | "mdown" | "mkd" => Style::Markdown,
        "txt" | "text" | "rst" | "adoc" | "asciidoc" | "org" => Style::Paragraphs,
        "ini" | "cfg" | "conf" | "properties" | "desktop" | "service" | "timer" | "socket"
        | "inf" | "editorconfig" | "gitconfig" | "gitmodules" => Style::Sections,
        _ if matches!(
            name.as_str(),
            ".editorconfig" | ".gitconfig" | ".gitmodules"
        ) =>
        {
            Style::Sections
        }
        _ => Style::Blocks,
    }
}

/// Every text file can be merged by its layout.
#[must_use]
pub fn supports(_path: &str) -> bool {
    true
}

/// A file split into keyed blocks: line ranges, plus the lines before the
/// first block and after the last.
#[derive(Debug)]
struct Layout {
    head: (usize, usize),
    blocks: Vec<(String, usize, usize)>,
    tail: (usize, usize),
}

fn content(line: &str) -> &str {
    line.trim_end_matches(['\n', '\r'])
}

fn is_blank(line: &str) -> bool {
    line.trim().is_empty()
}

fn indent(line: &str) -> usize {
    let t = content(line);
    t.len() - t.trim_start().len()
}

fn is_comment(line: &str) -> bool {
    let t = line.trim_start();
    ["#", "//", "--", ";", "/*", "*", "<!--", "%", "'"]
        .iter()
        .any(|p| t.starts_with(p))
}

/// A line that ends or continues something opened above it.
fn is_closer(line: &str) -> bool {
    let t = line.trim_start();
    if t.starts_with(['}', ']', ')']) || t.starts_with("</") {
        return true;
    }
    const WORDS: [&str; 17] = [
        "end",
        "fi",
        "done",
        "esac",
        "endif",
        "endfor",
        "endwhile",
        "endfunction",
        "endmodule",
        "elif",
        "else",
        "elsif",
        "elseif",
        "except",
        "finally",
        "catch",
        "rescue",
    ];
    WORDS.iter().any(|w| {
        t.strip_prefix(w)
            .is_some_and(|rest| !rest.starts_with(|c: char| c.is_alphanumeric() || c == '_'))
    })
}

/// How a line changes the bracket depth, outside quotes.
fn depth_change(line: &str) -> i64 {
    let mut depth = 0;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for c in content(line).chars() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '`' => quote = Some(c),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
    }
    depth
}

/// The key of a block from its first line: up to the first `(`, `{`, `=`
/// or `:` after its start, so a changed value or signature is the same
/// block. Headers (`[x]`, `<x>`) are kept whole.
fn line_key(line: &str) -> String {
    let t = content(line).trim();
    let key = if t.starts_with(['[', '<']) {
        t
    } else {
        let cut = t
            .char_indices()
            .skip(1)
            .find(|(_, c)| "({=:".contains(*c))
            .map_or(t.len(), |(i, _)| i);
        t[..cut].trim_end()
    };
    key.chars().take(MAX_KEY).collect()
}

/// Gives repeated keys the next line's key, then a number.
fn unique(blocks: &mut [(String, usize, usize)], lines: &[&str]) {
    let mut count = std::collections::HashMap::new();
    for (k, ..) in blocks.iter() {
        *count.entry(k.clone()).or_insert(0) += 1;
    }
    for (k, start, end) in blocks.iter_mut() {
        if count.get(k).copied().unwrap_or(0) > 1 {
            let next = lines[*start..*end]
                .iter()
                .filter(|l| !is_blank(l) && !is_comment(l))
                .nth(1)
                .map(|l| line_key(l))
                .unwrap_or_default();
            k.push('|');
            k.push_str(&next);
        }
    }
    let mut seen = std::collections::HashMap::new();
    for (k, ..) in blocks.iter_mut() {
        let n = seen.entry(k.clone()).or_insert(0);
        *n += 1;
        if *n > 1 {
            k.push_str(&format!("#{n}"));
        }
    }
}

/// Blocks by layout; `None` if brackets do not balance.
fn split_blocks(lines: &[&str]) -> Option<Layout> {
    let mut heads = Vec::new();
    let mut depth = 0i64;
    let mut previous: Option<&str> = None;
    for (i, line) in lines.iter().enumerate() {
        if is_blank(line) {
            continue;
        }
        let comment = is_comment(line);
        let continued = previous.is_some_and(|p| {
            let p = content(p).trim_end();
            p.ends_with('\\') || p.ends_with(',')
        });
        if indent(line) == 0 && depth == 0 && !is_closer(line) && !comment && !continued {
            heads.push(i);
        }
        if !comment {
            depth += depth_change(line);
            if depth < 0 {
                return None;
            }
            previous = Some(line);
        }
    }
    if depth != 0 || heads.is_empty() {
        return None;
    }
    // Comments and blank lines just above a block are its own.
    let mut starts = vec![0];
    for &h in &heads[1..] {
        let mut s = h;
        while s > 0
            && (is_blank(lines[s - 1]) || (indent(lines[s - 1]) == 0 && is_comment(lines[s - 1])))
        {
            s -= 1;
        }
        starts.push(s.max(*starts.last().unwrap_or(&0) + 1));
    }
    let last = lines
        .iter()
        .rposition(|l| !is_blank(l))
        .map_or(0, |i| i + 1);
    let mut blocks = Vec::new();
    for (n, &start) in starts.iter().enumerate() {
        let end = starts.get(n + 1).copied().unwrap_or(last);
        blocks.push((line_key(lines[heads[n]]), start, end.max(start)));
    }
    unique(&mut blocks, lines);
    Some(Layout {
        head: (0, 0),
        blocks,
        tail: (last, lines.len()),
    })
}

fn heading_level(line: &str) -> Option<usize> {
    let t = content(line);
    let hashes = t.bytes().take_while(|&b| b == b'#').count();
    let rest = &t[hashes..];
    ((1..=6).contains(&hashes) && (rest.is_empty() || rest.starts_with(' '))).then_some(hashes)
}

fn is_fence(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("```") || t.starts_with("~~~")
}

fn is_list_item(line: &str) -> bool {
    let t = content(line);
    if indent(line) != 0 {
        return false;
    }
    if t.starts_with("- ") || t.starts_with("* ") || t.starts_with("+ ") {
        return true;
    }
    let digits = t.bytes().take_while(u8::is_ascii_digit).count();
    digits > 0 && (t[digits..].starts_with(". ") || t[digits..].starts_with(") "))
}

/// Markdown: sections at the highest heading level, else paragraphs.
fn split_markdown(lines: &[&str]) -> Option<Layout> {
    let mut fenced = false;
    let mut headings = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if is_fence(line) {
            fenced = !fenced;
        } else if !fenced && let Some(level) = heading_level(line) {
            headings.push((i, level));
        }
    }
    if fenced {
        return None;
    }
    let Some(top) = headings.iter().map(|(_, l)| *l).min() else {
        return split_paragraphs(lines);
    };
    let starts: Vec<usize> = headings
        .iter()
        .filter(|(_, l)| *l == top)
        .map(|(i, _)| *i)
        .collect();
    let last = lines
        .iter()
        .rposition(|l| !is_blank(l))
        .map_or(0, |i| i + 1);
    let mut blocks = Vec::new();
    for (n, &start) in starts.iter().enumerate() {
        let end = starts.get(n + 1).copied().unwrap_or(last).max(start);
        let key = content(lines[start])
            .trim()
            .trim_end_matches('#')
            .trim()
            .to_owned();
        blocks.push((key, start, end));
    }
    unique(&mut blocks, lines);
    Some(Layout {
        head: (0, starts[0]),
        blocks,
        tail: (last, lines.len()),
    })
}

/// Paragraphs (separated by blank lines) and list items, each a block.
fn split_paragraphs(lines: &[&str]) -> Option<Layout> {
    let mut starts = Vec::new();
    let mut fenced = false;
    let mut after_blank = true;
    for (i, line) in lines.iter().enumerate() {
        if fenced {
            if is_fence(line) {
                fenced = false;
            }
            continue;
        }
        if is_blank(line) {
            after_blank = true;
            continue;
        }
        if after_blank || is_list_item(line) || is_fence(line) {
            starts.push(i);
        }
        if is_fence(line) {
            fenced = true;
        }
        after_blank = false;
    }
    if fenced || starts.is_empty() {
        return None;
    }
    // Blank lines before a paragraph are its own.
    let mut begins = vec![0];
    for &s in &starts[1..] {
        let mut b = s;
        while b > 0 && is_blank(lines[b - 1]) {
            b -= 1;
        }
        begins.push(b.max(*begins.last().unwrap_or(&0) + 1));
    }
    let last = lines
        .iter()
        .rposition(|l| !is_blank(l))
        .map_or(0, |i| i + 1);
    let mut blocks = Vec::new();
    for (n, &begin) in begins.iter().enumerate() {
        let end = begins.get(n + 1).copied().unwrap_or(last).max(begin);
        let key: String = content(lines[starts[n]])
            .trim()
            .chars()
            .take(MAX_KEY)
            .collect();
        blocks.push((key, begin, end));
    }
    unique(&mut blocks, lines);
    Some(Layout {
        head: (0, 0),
        blocks,
        tail: (last, lines.len()),
    })
}

/// INI-style files: one block per `[section]`, else per key line.
fn split_sections(lines: &[&str]) -> Option<Layout> {
    let header = |l: &str| {
        let t = content(l).trim();
        t.starts_with('[') && t.ends_with(']')
    };
    let starts: Vec<usize> = (0..lines.len()).filter(|&i| header(lines[i])).collect();
    if starts.is_empty() {
        return split_blocks(lines);
    }
    let last = lines
        .iter()
        .rposition(|l| !is_blank(l))
        .map_or(0, |i| i + 1);
    let mut blocks = Vec::new();
    for (n, &start) in starts.iter().enumerate() {
        let end = starts.get(n + 1).copied().unwrap_or(last).max(start);
        blocks.push((content(lines[start]).trim().to_owned(), start, end));
    }
    unique(&mut blocks, lines);
    Some(Layout {
        head: (0, starts[0]),
        blocks,
        tail: (last, lines.len()),
    })
}

fn split(style: Style, lines: &[&str]) -> Option<Layout> {
    match style {
        Style::Blocks => split_blocks(lines),
        Style::Markdown => split_markdown(lines),
        Style::Paragraphs => split_paragraphs(lines),
        Style::Sections => split_sections(lines),
    }
}

fn join(lines: &[&str]) -> String {
    lines.concat()
}

fn lines_of(text: &str) -> Vec<&str> {
    text.split_inclusive('\n').collect()
}

fn merge_lines(style: Style, b: &[&str], o: &[&str], t: &[&str], depth: usize) -> Option<String> {
    let (lb, lo, lt) = (split(style, b)?, split(style, o)?, split(style, t)?);
    let range = |lines: &[&str], (s, e): (usize, usize)| join(&lines[s..e]);
    let head = merge_text(
        Some(&range(b, lb.head)),
        &range(o, lo.head),
        &range(t, lt.head),
    )?;
    let tail = merge_text(
        Some(&range(b, lb.tail)),
        &range(o, lo.tail),
        &range(t, lt.tail),
    )?;
    let pieces = |lines: &[&str], l: &Layout| -> Vec<Piece> {
        l.blocks
            .iter()
            .map(|(key, s, e)| Piece {
                key: key.clone(),
                text: join(&lines[*s..*e]),
            })
            .collect()
    };
    let texts = merge_pieces(
        &pieces(b, &lb),
        &pieces(o, &lo),
        &pieces(t, &lt),
        &mut |bi, oi, ti| {
            if depth >= MAX_NESTING {
                return None;
            }
            let block = |lines: &'_ [&'_ str], l: &Layout, i: usize| -> Vec<String> {
                let (_, s, e) = &l.blocks[i];
                lines[*s..*e].iter().map(|l| (*l).to_owned()).collect()
            };
            nested(
                style,
                &block(b, &lb, bi),
                &block(o, &lo, oi),
                &block(t, &lt, ti),
                depth + 1,
            )
        },
    )?;
    Some(format!("{head}{}{tail}", texts.concat()))
}

/// A block cut into its first line, what is indented under it, and the
/// lines that close it.
struct Shape<'a> {
    head: String,
    body: Vec<&'a str>,
    foot: String,
    indent: String,
}

fn shape(style: Style, lines: &[String]) -> Option<Shape<'_>> {
    // In Markdown a `#` starts a heading, not a comment.
    let first = lines
        .iter()
        .position(|l| !is_blank(l) && (style != Style::Blocks || !is_comment(l)))?;
    let mut end = lines.len();
    if style == Style::Blocks {
        while end > first + 1
            && (is_blank(&lines[end - 1])
                || (indent(&lines[end - 1]) == 0 && is_closer(&lines[end - 1])))
        {
            end -= 1;
        }
    }
    let body: Vec<&str> = lines[first + 1..end].iter().map(String::as_str).collect();
    let indent = if style == Style::Blocks {
        let min = body
            .iter()
            .filter(|l| !is_blank(l))
            .map(|l| indent(l))
            .min()?;
        if min == 0 {
            return None;
        }
        let model = body.iter().find(|l| !is_blank(l))?;
        let prefix = &model[..min];
        if !body.iter().all(|l| is_blank(l) || l.starts_with(prefix)) {
            return None;
        }
        prefix.to_owned()
    } else {
        String::new()
    };
    Some(Shape {
        head: lines[..=first].concat(),
        body,
        foot: lines[end..].concat(),
        indent,
    })
}

fn dedent(s: &Shape<'_>) -> Vec<String> {
    s.body
        .iter()
        .map(|l| {
            if is_blank(l) {
                (*l).to_owned()
            } else {
                l[s.indent.len()..].to_owned()
            }
        })
        .collect()
}

fn view(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}

/// Both sides changed one block: merge what is under its first line.
fn nested(style: Style, b: &[String], o: &[String], t: &[String], depth: usize) -> Option<String> {
    let (sb, so, st) = (shape(style, b)?, shape(style, o)?, shape(style, t)?);
    if sb.indent != so.indent || so.indent != st.indent {
        return None;
    }
    let head = merge_text(Some(&sb.head), &so.head, &st.head)?;
    let foot = merge_text(Some(&sb.foot), &so.foot, &st.foot)?;
    let (db, dob, dt) = (dedent(&sb), dedent(&so), dedent(&st));
    let inner_style = match style {
        Style::Sections => Style::Blocks,
        other => other,
    };
    let inner = merge_lines(inner_style, &view(&db), &view(&dob), &view(&dt), depth)?;
    let mut body = String::new();
    for line in inner.split_inclusive('\n') {
        if !is_blank(line) {
            body.push_str(&so.indent);
        }
        body.push_str(line);
    }
    Some(format!("{head}{body}{foot}"))
}

/// Merges by layout; `None` when the layout does not tell the parts apart
/// or both sides changed the same part.
#[must_use]
pub fn merge(path: &str, base: &str, ours: &str, theirs: &str) -> Option<String> {
    let style = style(path);
    merge_lines(
        style,
        &lines_of(base),
        &lines_of(ours),
        &lines_of(theirs),
        0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_added_at_the_end_merge_in_any_language() {
        let base = "fun one(): Int {\n    return 1\n}\n";
        let ours = format!("{base}\nfun two(): Int {{\n    return 2\n}}\n");
        let theirs = format!("{base}\nfun three(): Int {{\n    return 3\n}}\n");
        let merged = merge("x.unknownlang", base, &ours, &theirs).unwrap();
        assert_eq!(
            merged,
            format!(
                "{base}\nfun two(): Int {{\n    return 2\n}}\n\nfun three(): Int {{\n    return 3\n}}\n"
            )
        );
    }

    #[test]
    fn edits_inside_one_block_merge_one_level_down() {
        let base = "services:\n  web:\n    image: a\n";
        let ours = "services:\n  web:\n    image: a\n  db:\n    image: postgres\n";
        let theirs = "services:\n  web:\n    image: a\n  cache:\n    image: redis\n";
        let merged = merge("compose.conf.unknown", base, ours, theirs).unwrap();
        assert!(
            merged.contains("  db:\n") && merged.contains("  cache:\n"),
            "{merged}"
        );
    }

    #[test]
    fn changelog_entries_under_one_heading_merge() {
        let base = "# Changelog\n\n## Unreleased\n\n- first\n\n## 1.0\n\n- old\n";
        let ours = "# Changelog\n\n## Unreleased\n\n- first\n- from ana\n\n## 1.0\n\n- old\n";
        let theirs =
            "# Changelog\n\n## Unreleased\n\n- first\n- from the agent\n\n## 1.0\n\n- old\n";
        let merged = merge("CHANGELOG.md", base, ours, theirs).unwrap();
        assert!(
            merged.contains("- first\n- from ana\n- from the agent\n"),
            "{merged}"
        );
    }

    #[test]
    fn dockerfile_and_ini_merge() {
        let base = "FROM rust AS build\nRUN cargo build\n";
        let ours = "FROM rust AS build\nRUN cargo build\nRUN strip target/app\n";
        let theirs = "FROM rust AS build\nRUN cargo build\nEXPOSE 8080\n";
        let merged = merge("Dockerfile", base, ours, theirs).unwrap();
        assert!(
            merged.contains("RUN strip") && merged.contains("EXPOSE 8080"),
            "{merged}"
        );

        let base = "[core]\nbare = false\n";
        let ours = "[core]\nbare = false\nautocrlf = input\n";
        let theirs = "[core]\nbare = false\nfilemode = true\n";
        let merged = merge("config.ini", base, ours, theirs).unwrap();
        assert!(
            merged.contains("autocrlf") && merged.contains("filemode"),
            "{merged}"
        );
    }

    #[test]
    fn conflicts_and_unbalanced_files_are_left_alone() {
        let base = "key: 1\n";
        assert!(merge("a.cfgx", base, "key: 2\n", "key: 3\n").is_none());
        assert!(merge("a.x", "f(\n", "f(\na\n", "f(\nb\n").is_none());
        let fence = "# T\n\n```\ncode\n";
        assert!(
            merge(
                "a.md",
                fence,
                &format!("{fence}a\n"),
                &format!("{fence}b\n")
            )
            .is_none()
        );
    }
}
