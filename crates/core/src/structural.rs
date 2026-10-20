//! Structural merge: when a line merge conflicts, try again one top-level
//! declaration at a time (functions, types, imports, classes…).
//!
//! Two agents adding different functions at the end of a file, or editing
//! neighbouring functions, conflict line by line because their hunks touch.
//! Here each file is split, with tree-sitter, into keyed declarations
//! (kind + name); each declaration merges on its own, new ones keep their
//! place, and the result must parse without errors or the conflict stands.

use std::collections::{HashMap, HashSet};

use tree_sitter::{Language, Node, Parser};

/// The grammar for a path, by extension.
fn language(path: &str) -> Option<Language> {
    let ext = path.rsplit_once('.').map(|(_, e)| e)?;
    Some(match ext {
        "rs" => tree_sitter_rust::LANGUAGE.into(),
        "ts" | "mts" | "cts" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        "js" | "mjs" | "cjs" | "jsx" => tree_sitter_javascript::LANGUAGE.into(),
        "py" | "pyi" => tree_sitter_python::LANGUAGE.into(),
        "go" => tree_sitter_go::LANGUAGE.into(),
        "java" => tree_sitter_java::LANGUAGE.into(),
        _ => return None,
    })
}

/// Whether a path is merged structurally when lines conflict.
#[must_use]
pub fn supports(path: &str) -> bool {
    language(path).is_some()
}

/// One top-level declaration with the text before it (blank lines and
/// comments belong to the declaration that follows them).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Item {
    key: String,
    text: String,
}

/// A file as keyed declarations plus whatever follows the last one.
#[derive(Debug)]
struct Split {
    items: Vec<Item>,
    tail: String,
}

fn parse(language: &Language, text: &str) -> Option<tree_sitter::Tree> {
    let mut parser = Parser::new();
    parser.set_language(language).ok()?;
    let tree = parser.parse(text, None)?;
    (!tree.root_node().has_error()).then_some(tree)
}

/// The name of a declaration: its `name` field, or that of the first
/// descendant that has one (decorated or exported definitions), or for
/// Rust `impl` blocks the trait and type.
fn name_of(node: Node<'_>, text: &str) -> Option<String> {
    let field = |n: Node<'_>, f: &str| {
        n.child_by_field_name(f)
            .and_then(|c| c.utf8_text(text.as_bytes()).ok())
            .map(str::to_owned)
    };
    if node.kind() == "impl_item" {
        let ty = field(node, "type")?;
        return Some(match field(node, "trait") {
            Some(tr) => format!("{tr} for {ty}"),
            None => ty,
        });
    }
    let mut queue = vec![(node, 0)];
    while let Some((n, depth)) = queue.pop() {
        if let Some(name) = field(n, "name") {
            return Some(name);
        }
        if depth < 3 {
            let mut cursor = n.walk();
            let children: Vec<Node<'_>> = n.named_children(&mut cursor).collect();
            queue.extend(children.into_iter().rev().map(|c| (c, depth + 1)));
        }
    }
    None
}

fn split(language: &Language, text: &str) -> Option<Split> {
    let tree = parse(language, text)?;
    let root = tree.root_node();
    let mut items = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut start = 0;
    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        // Comments travel with the declaration after them.
        if node.kind().contains("comment") {
            continue;
        }
        let end = node.end_byte();
        let decl = text.get(node.start_byte()..end)?;
        let base_key = match name_of(node, text) {
            Some(name) => format!("{}:{name}", node.kind()),
            // Unnamed (imports, statements): the text itself is the key.
            None => format!("{}={}", node.kind(), decl.trim()),
        };
        let n = seen.entry(base_key.clone()).or_insert(0);
        *n += 1;
        let key = if *n == 1 {
            base_key
        } else {
            format!("{base_key}#{n}")
        };
        items.push(Item {
            key,
            text: text.get(start..end)?.to_owned(),
        });
        start = end;
    }
    Some(Split {
        items,
        tail: text.get(start..)?.to_owned(),
    })
}

/// Three-way merge of one piece of text: one side's change wins if the
/// other left it alone; both changed means a line merge or nothing.
fn merge_text(base: Option<&str>, ours: &str, theirs: &str) -> Option<String> {
    if ours == theirs || Some(theirs) == base {
        return Some(ours.to_owned());
    }
    if Some(ours) == base {
        return Some(theirs.to_owned());
    }
    diffy::merge(base.unwrap_or(""), ours, theirs).ok()
}

/// Merges `ours` and `theirs` (both changed from `base`) declaration by
/// declaration, or returns `None` if that is not possible: an unknown
/// language, a parse error, a declaration both sides changed in
/// conflicting ways, or one side changed what the other deleted.
#[must_use]
pub fn merge(path: &str, base: &str, ours: &str, theirs: &str) -> Option<String> {
    let language = language(path)?;
    let (b, o, t) = (
        split(&language, base)?,
        split(&language, ours)?,
        split(&language, theirs)?,
    );
    let index = |s: &Split| -> HashMap<String, String> {
        s.items
            .iter()
            .map(|i| (i.key.clone(), i.text.clone()))
            .collect()
    };
    let (bi, oi, ti) = (index(&b), index(&o), index(&t));

    // Decide each declaration.
    let mut merged: HashMap<String, String> = HashMap::new();
    let keys: HashSet<&String> = bi.keys().chain(oi.keys()).chain(ti.keys()).collect();
    for key in keys {
        let (bx, ox, tx) = (bi.get(key), oi.get(key), ti.get(key));
        let result = match (ox, tx) {
            (Some(ox), Some(tx)) => Some(merge_text(bx.map(String::as_str), ox, tx)?),
            // Deleted on one side: fine only if the other did not touch it.
            (Some(kept), None) | (None, Some(kept)) => match bx {
                Some(bx) if bx == kept => None,
                Some(_) => return None,
                None => Some(kept.clone()),
            },
            (None, None) => None,
        };
        if let Some(text) = result {
            merged.insert(key.clone(), text);
        }
    }

    // Order: ours, with their new declarations after the one before them
    // in their file.
    let mut order: Vec<String> = o
        .items
        .iter()
        .map(|i| i.key.clone())
        .filter(|k| merged.contains_key(k))
        .collect();
    let mut previous: Option<String> = None;
    for item in &t.items {
        let new = !bi.contains_key(&item.key) && !oi.contains_key(&item.key);
        if new && merged.contains_key(&item.key) {
            let mut at = previous
                .as_ref()
                .and_then(|p| order.iter().position(|k| k == p))
                .map_or(0, |i| i + 1);
            // In the same spot, our new declarations come first.
            while at < order.len() && !bi.contains_key(&order[at]) && !ti.contains_key(&order[at]) {
                at += 1;
            }
            order.insert(at, item.key.clone());
        }
        if order.contains(&item.key) {
            previous = Some(item.key.clone());
        }
    }
    // Declarations kept from theirs that ours deleted cannot exist here
    // (that returned above), so `order` holds every merged declaration.

    let mut out = String::new();
    for key in &order {
        out.push_str(&merged[key]);
    }
    out.push_str(&merge_text(Some(&b.tail), &o.tail, &t.tail)?);
    parse(&language, &out)?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "use std::fs;\n\nfn one() -> u8 {\n    1\n}\n";

    #[test]
    fn both_sides_adding_functions_at_the_end_merge() {
        let ours = format!("{BASE}\nfn two() -> u8 {{\n    2\n}}\n");
        let theirs = format!("{BASE}\nfn three() -> u8 {{\n    3\n}}\n");
        // Lines alone cannot do it: both inserted at the same place.
        assert!(diffy::merge(BASE, &ours, &theirs).is_err());
        let merged = merge("lib.rs", BASE, &ours, &theirs).unwrap();
        assert_eq!(
            merged,
            format!("{BASE}\nfn two() -> u8 {{\n    2\n}}\n\nfn three() -> u8 {{\n    3\n}}\n")
        );
    }

    #[test]
    fn neighbouring_edits_and_new_imports_merge() {
        let base = "import a from 'a';\n\nexport function f() {\n  return 1;\n}\nexport function g() {\n  return 2;\n}\n";
        let ours = "import a from 'a';\nimport b from 'b';\n\nexport function f() {\n  return 10;\n}\nexport function g() {\n  return 2;\n}\n";
        let theirs = "import a from 'a';\nimport c from 'c';\n\nexport function f() {\n  return 1;\n}\nexport function g() {\n  return 20;\n}\n";
        let merged = merge("x.ts", base, ours, theirs).unwrap();
        assert!(merged.contains("import b from 'b';"));
        assert!(merged.contains("import c from 'c';"));
        assert!(merged.contains("return 10;"));
        assert!(merged.contains("return 20;"));
    }

    #[test]
    fn real_conflicts_and_unknown_files_are_left_alone() {
        let ours = "fn one() -> u8 {\n    10\n}\n";
        let theirs = "fn one() -> u8 {\n    20\n}\n";
        assert!(merge("lib.rs", BASE, ours, theirs).is_none());
        assert!(merge("notes.txt", "a\n", "b\n", "c\n").is_none());
        // Edited on one side, deleted on the other.
        let edited = BASE.replace("    1", "    100");
        assert!(merge("lib.rs", BASE, &edited, "use std::fs;\n").is_none());
    }

    #[test]
    fn python_decorated_definitions_keep_their_names() {
        let base = "import os\n\n\n@cache\ndef a():\n    return 1\n";
        let ours = format!("{base}\n\ndef b():\n    return 2\n");
        let theirs = format!("{base}\n\ndef c():\n    return 3\n");
        let merged = merge("m.py", base, &ours, &theirs).unwrap();
        assert!(merged.contains("def b():") && merged.contains("def c():"));
        assert!(merged.find("def b").unwrap() < merged.find("def c").unwrap());
    }
}
