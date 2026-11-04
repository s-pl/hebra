//! Structural merge: when a line merge conflicts, try again by named parts
//! of the file (declarations, class members, object keys, sections).
//!
//! A file is split into keyed parts, with a tree-sitter grammar for some
//! thirty languages (functions, types, methods, fields, JSON and YAML keys,
//! TOML tables…) and, for any other text, by its layout (see
//! [`crate::layout`]). Each part merges on its own. When both sides changed
//! the same part, its body is merged the same way one level down: two
//! methods added to one class, two dependencies added to one table. New
//! parts keep their place, and separators (commas) are put back where they
//! belong. The result must parse without errors, or the conflict stands.
//!
//! Only containers of named parts (declarations, keys) are merged inside:
//! statements in a function body keep their order, so two edits there stay
//! a conflict for a person or an agent to settle.

use std::collections::{HashMap, HashSet, VecDeque};

use tree_sitter::{Language, Node, Parser};

/// Files larger than this are only merged by lines.
const MAX_BYTES: usize = 4 * 1024 * 1024;
/// How many levels a merge goes into nested parts.
const MAX_NESTING: usize = 8;
/// Names and keys longer than this are not names.
const MAX_NAME: usize = 200;

/// Code keeps statement order; data (JSON, YAML, TOML, HCL) is keyed all
/// the way down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Code,
    Data,
}

/// The grammar for a path, by file name or extension.
fn grammar(path: &str) -> Option<(Language, Family)> {
    use Family::{Code, Data};
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    let by_name = match name.as_str() {
        "gemfile" | "rakefile" | "podfile" | "vagrantfile" | "brewfile" | "guardfile" => {
            Some((tree_sitter_ruby::LANGUAGE.into(), Code))
        }
        "cargo.lock" | "pipfile" | "poetry.lock" | "uv.lock" | "pdm.lock" => {
            Some((tree_sitter_toml_ng::LANGUAGE.into(), Data))
        }
        ".babelrc" | ".eslintrc" | ".prettierrc" | ".swcrc" | "deno.lock" | "flake.lock" => {
            Some((tree_sitter_json::LANGUAGE.into(), Data))
        }
        _ => None,
    };
    if by_name.is_some() {
        return by_name;
    }
    let ext = name.rsplit_once('.').map(|(_, e)| e)?;
    let language: Language = match ext {
        "rs" => tree_sitter_rust::LANGUAGE.into(),
        "ts" | "mts" | "cts" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        "js" | "mjs" | "cjs" | "jsx" => tree_sitter_javascript::LANGUAGE.into(),
        "py" | "pyi" | "pyw" => tree_sitter_python::LANGUAGE.into(),
        "go" => tree_sitter_go::LANGUAGE.into(),
        "java" => tree_sitter_java::LANGUAGE.into(),
        "c" => tree_sitter_c::LANGUAGE.into(),
        #[cfg(feature = "extra-grammars")]
        "h" | "cc" | "cpp" | "cxx" | "c++" | "hpp" | "hh" | "hxx" | "h++" | "ipp" | "tpp"
        | "ino" => tree_sitter_cpp::LANGUAGE.into(),
        #[cfg(feature = "extra-grammars")]
        "cs" => tree_sitter_c_sharp::LANGUAGE.into(),
        "rb" | "rake" | "gemspec" | "ru" => tree_sitter_ruby::LANGUAGE.into(),
        "php" | "phtml" => tree_sitter_php::LANGUAGE_PHP.into(),
        "sh" | "bash" | "zsh" | "ksh" | "bats" => tree_sitter_bash::LANGUAGE.into(),
        "css" => tree_sitter_css::LANGUAGE.into(),
        "html" | "htm" | "xhtml" => tree_sitter_html::LANGUAGE.into(),
        #[cfg(feature = "extra-grammars")]
        "scala" | "sc" | "sbt" => tree_sitter_scala::LANGUAGE.into(),
        "lua" => tree_sitter_lua::LANGUAGE.into(),
        #[cfg(feature = "extra-grammars")]
        "kt" | "kts" => tree_sitter_kotlin_ng::LANGUAGE.into(),
        #[cfg(feature = "extra-grammars")]
        "swift" => tree_sitter_swift::LANGUAGE.into(),
        #[cfg(feature = "extra-grammars")]
        "hs" => tree_sitter_haskell::LANGUAGE.into(),
        #[cfg(feature = "extra-grammars")]
        "ex" | "exs" => tree_sitter_elixir::LANGUAGE.into(),
        #[cfg(feature = "extra-grammars")]
        "zig" => tree_sitter_zig::LANGUAGE.into(),
        #[cfg(feature = "extra-grammars")]
        "ml" => tree_sitter_ocaml::LANGUAGE_OCAML.into(),
        #[cfg(feature = "extra-grammars")]
        "mli" => tree_sitter_ocaml::LANGUAGE_OCAML_INTERFACE.into(),
        #[cfg(feature = "extra-grammars")]
        "dart" => tree_sitter_dart::LANGUAGE.into(),
        #[cfg(feature = "extra-grammars")]
        "sol" => tree_sitter_solidity::LANGUAGE.into(),
        "proto" => tree_sitter_proto::LANGUAGE.into(),
        #[cfg(feature = "extra-grammars")]
        "r" => tree_sitter_r::LANGUAGE.into(),
        #[cfg(feature = "extra-grammars")]
        "jl" => tree_sitter_julia::LANGUAGE.into(),
        "nix" => tree_sitter_nix::LANGUAGE.into(),
        #[cfg(feature = "extra-grammars")]
        "m" | "mm" => tree_sitter_objc::LANGUAGE.into(),
        #[cfg(feature = "extra-grammars")]
        "ps1" | "psm1" | "psd1" => tree_sitter_powershell::LANGUAGE.into(),
        #[cfg(feature = "extra-grammars")]
        "elm" => tree_sitter_elm::LANGUAGE.into(),
        #[cfg(feature = "extra-grammars")]
        "erl" | "hrl" => tree_sitter_erlang::LANGUAGE.into(),
        #[cfg(feature = "extra-grammars")]
        "gleam" => tree_sitter_gleam::LANGUAGE.into(),
        "json" | "jsonc" | "geojson" | "webmanifest" | "code-workspace" => {
            return Some((tree_sitter_json::LANGUAGE.into(), Data));
        }
        "yml" | "yaml" => return Some((tree_sitter_yaml::LANGUAGE.into(), Data)),
        "toml" => return Some((tree_sitter_toml_ng::LANGUAGE.into(), Data)),
        "tf" | "tfvars" | "hcl" | "nomad" => {
            return Some((tree_sitter_hcl::LANGUAGE.into(), Data));
        }
        _ => return None,
    };
    Some((language, Code))
}

/// Whether a path is merged structurally when lines conflict: with a
/// grammar, or by its layout (any text).
#[must_use]
pub fn supports(path: &str) -> bool {
    grammar(path).is_some() || crate::layout::supports(path)
}

/// Whether a path has a grammar (for `hb` to say how it merges).
#[must_use]
pub fn has_grammar(path: &str) -> bool {
    grammar(path).is_some()
}

fn parse(language: &Language, text: &str) -> Option<tree_sitter::Tree> {
    let mut parser = Parser::new();
    parser.set_language(language).ok()?;
    let tree = parser.parse(text, None)?;
    (!tree.root_node().has_error()).then_some(tree)
}

/// One keyed part of a container, with the text it owns.
#[derive(Debug, Clone)]
struct Part<'t> {
    key: String,
    /// Whether the key is a name (a declaration, a key) rather than text.
    named: bool,
    start: usize,
    end: usize,
    node: Node<'t>,
}

/// A node's parts, the text before the first and after the last, and the
/// separator between them when there is one (`,` in objects and lists).
#[derive(Debug)]
struct Container<'t> {
    head: (usize, usize),
    parts: Vec<Part<'t>>,
    tail: (usize, usize),
    sep: Option<String>,
    trailing: bool,
}

fn is_comment(node: &Node<'_>) -> bool {
    node.kind().contains("comment")
}

fn one_line(text: &str) -> Option<String> {
    let t = text.trim();
    (!t.is_empty() && t.len() <= MAX_NAME && !t.contains('\n')).then(|| t.to_owned())
}

/// The name of a declaration: a `name` or `key` field, or a C-style
/// declarator, on the node or on a descendant close to it (decorated or
/// exported definitions). Rust `impl` blocks are named by trait and type.
fn name_of(node: Node<'_>, text: &str) -> Option<String> {
    let field = |n: Node<'_>, f: &str| {
        n.child_by_field_name(f)
            .and_then(|c| c.utf8_text(text.as_bytes()).ok())
            .and_then(one_line)
    };
    if node.kind() == "impl_item" {
        let ty = field(node, "type")?;
        return Some(match field(node, "trait") {
            Some(tr) => format!("{tr} for {ty}"),
            None => ty,
        });
    }
    let mut queue = VecDeque::from([(node, 0)]);
    while let Some((n, depth)) = queue.pop_front() {
        if let Some(name) = field(n, "name").or_else(|| field(n, "key")) {
            return Some(name);
        }
        if let Some(d) = n.child_by_field_name("declarator") {
            if d.kind().contains("identifier") {
                return d.utf8_text(text.as_bytes()).ok().and_then(one_line);
            }
            queue.push_front((d, depth + 1));
            continue;
        }
        if depth < 3 {
            let mut cursor = n.walk();
            for c in n.named_children(&mut cursor) {
                if !is_comment(&c) {
                    queue.push_back((c, depth + 1));
                }
            }
        }
    }
    None
}

fn named_children<'t>(node: Node<'t>) -> Vec<Node<'t>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter(|c| !is_comment(c))
        .collect()
}

/// Keys for the parts of one container: names where there are any, else
/// the first child's text (an import's path, a selector, a table's name),
/// else the text itself. Repeated keys take the next child's text, then a
/// number.
fn keys(nodes: &[Node<'_>], text: &str) -> Vec<(String, bool)> {
    let src = |n: Node<'_>| n.utf8_text(text.as_bytes()).unwrap_or_default();
    let mut out: Vec<(String, bool)> = nodes
        .iter()
        .map(|n| match name_of(*n, text) {
            Some(name) => (format!("{}:{name}", n.kind()), true),
            None => {
                let first = named_children(*n)
                    .first()
                    .and_then(|c| one_line(src(*c)))
                    .unwrap_or_else(|| src(*n).trim().to_owned());
                (format!("{}={first}", n.kind()), false)
            }
        })
        .collect();
    for extra in 1..3 {
        let mut count: HashMap<&str, usize> = HashMap::new();
        for (k, _) in &out {
            *count.entry(k.as_str()).or_default() += 1;
        }
        let repeated: HashSet<String> = count
            .into_iter()
            .filter(|(_, n)| *n > 1)
            .map(|(k, _)| k.to_owned())
            .collect();
        if repeated.is_empty() {
            return out;
        }
        for (i, (k, _)) in out.iter_mut().enumerate() {
            if repeated.contains(k.as_str()) {
                let more = named_children(nodes[i])
                    .get(extra)
                    .and_then(|c| one_line(src(*c)))
                    .unwrap_or_default();
                k.push('|');
                k.push_str(&more);
            }
        }
    }
    let mut seen: HashMap<String, usize> = HashMap::new();
    for (k, _) in &mut out {
        let n = seen.entry(k.clone()).or_insert(0);
        *n += 1;
        if *n > 1 {
            k.push_str(&format!("#{n}"));
        }
    }
    out
}

/// Splits `node` into parts. At the top (`whole`), the container is the
/// whole file.
fn split<'t>(node: Node<'t>, text: &str, whole: bool) -> Option<Container<'t>> {
    let mut cursor = node.walk();
    let children: Vec<Node<'t>> = node.children(&mut cursor).collect();
    let items: Vec<usize> = (0..children.len())
        .filter(|&i| children[i].is_named() && !is_comment(&children[i]))
        .collect();
    let first = *items.first()?;
    let last = *items.last()?;
    let (start, end) = if whole {
        (0, text.len())
    } else {
        (node.start_byte(), node.end_byte())
    };
    let head_end = if whole {
        0
    } else {
        children[..first]
            .iter()
            .filter(|c| !c.is_named())
            .map(Node::end_byte)
            .max()
            .unwrap_or(start)
    };
    // A separator: exactly one `,` or `;` between every two parts.
    let gaps: Vec<Vec<Node<'t>>> = items
        .windows(2)
        .map(|w| {
            children[w[0] + 1..w[1]]
                .iter()
                .filter(|c| !c.is_named())
                .copied()
                .collect()
        })
        .collect();
    let token = |n: &Node<'t>| n.utf8_text(text.as_bytes()).unwrap_or_default().to_owned();
    let sep = gaps.first().and_then(|g| g.first()).map(token).filter(|s| {
        (s == "," || s == ";") && gaps.iter().all(|g| g.len() == 1 && token(&g[0]) == *s)
    });
    let after_last: Vec<Node<'t>> = children[last + 1..]
        .iter()
        .filter(|c| !c.is_named())
        .copied()
        .collect();
    let trailing = sep
        .as_ref()
        .is_some_and(|s| after_last.first().is_some_and(|c| token(c) == *s));

    let nodes: Vec<Node<'t>> = items.iter().map(|&i| children[i]).collect();
    let keys = keys(&nodes, text);
    let mut parts = Vec::with_capacity(items.len());
    let mut at = head_end;
    for (n, &i) in items.iter().enumerate() {
        let node = children[i];
        let part_end = match (&sep, gaps.get(n)) {
            // Up to the separator: comments before it stay with the part.
            (Some(_), Some(gap)) => gap[0].start_byte(),
            (Some(_), None) if trailing => after_last[0].start_byte(),
            (Some(_), None) => node.end_byte(),
            // Tokens after a part (a `;`) stay with it.
            (None, Some(gap)) => gap
                .iter()
                .map(Node::end_byte)
                .max()
                .unwrap_or(node.end_byte()),
            (None, None) => node.end_byte(),
        };
        let (key, named) = keys[n].clone();
        parts.push(Part {
            key,
            named,
            start: at,
            end: part_end,
            node,
        });
        at = match (&sep, gaps.get(n)) {
            (Some(_), Some(gap)) => gap[0].end_byte(),
            (Some(_), None) if trailing => after_last[0].end_byte(),
            _ => part_end,
        };
    }
    Some(Container {
        head: (start, head_end),
        parts,
        tail: (at, end),
        sep,
        trailing,
    })
}

/// Three-way merge of one piece of text: one side's change wins if the
/// other left it alone; both changed means a line merge or nothing.
pub(crate) fn merge_text(base: Option<&str>, ours: &str, theirs: &str) -> Option<String> {
    if ours == theirs || Some(theirs) == base {
        return Some(ours.to_owned());
    }
    if Some(ours) == base {
        return Some(theirs.to_owned());
    }
    diffy::merge(base.unwrap_or(""), ours, theirs).ok()
}

/// A keyed piece of a file, as both mergers see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Piece {
    pub key: String,
    pub text: String,
}

/// Merges keyed pieces. `nested(b, o, t)` is asked, with the index of the
/// piece on each side, when both sides changed one piece and its lines
/// conflict. Returns the merged texts in order: ours, with their new
/// pieces after the piece before them in their version.
pub(crate) fn merge_pieces(
    b: &[Piece],
    o: &[Piece],
    t: &[Piece],
    nested: &mut dyn FnMut(usize, usize, usize) -> Option<String>,
) -> Option<Vec<String>> {
    let index = |s: &[Piece]| -> HashMap<String, usize> {
        s.iter()
            .enumerate()
            .map(|(i, p)| (p.key.clone(), i))
            .collect()
    };
    let (bi, oi, ti) = (index(b), index(o), index(t));
    // A piece gone from both sides with something new in its place on
    // each (a paragraph or a name rewritten twice) is a conflict, not two
    // additions.
    for (i, piece) in b.iter().enumerate() {
        if oi.contains_key(&piece.key) || ti.contains_key(&piece.key) {
            continue;
        }
        let slot = |side: &[Piece], at: &HashMap<String, usize>| -> bool {
            let before = b[..i]
                .iter()
                .rev()
                .find_map(|p| at.get(&p.key))
                .map_or(0, |&p| p + 1);
            let after = b[i + 1..]
                .iter()
                .find_map(|p| at.get(&p.key))
                .copied()
                .unwrap_or(side.len());
            side.get(before..after.max(before))
                .is_some_and(|s| s.iter().any(|p| !bi.contains_key(&p.key)))
        };
        if slot(o, &oi) && slot(t, &ti) {
            return None;
        }
    }
    let mut merged: HashMap<String, String> = HashMap::new();
    let keys: HashSet<&String> = bi.keys().chain(oi.keys()).chain(ti.keys()).collect();
    for key in keys {
        let bx = bi.get(key).map(|&i| b[i].text.as_str());
        let result = match (oi.get(key), ti.get(key)) {
            (Some(&ox), Some(&tx)) => match merge_text(bx, &o[ox].text, &t[tx].text) {
                Some(text) => Some(text),
                None => Some(nested(*bi.get(key)?, ox, tx)?),
            },
            // Deleted on one side: fine only if the other did not touch it.
            (Some(&i), None) => keep_or_drop(bx, &o[i].text)?,
            (None, Some(&i)) => keep_or_drop(bx, &t[i].text)?,
            (None, None) => None,
        };
        if let Some(text) = result {
            merged.insert(key.clone(), text);
        }
    }
    let mut order: Vec<String> = o
        .iter()
        .map(|p| p.key.clone())
        .filter(|k| merged.contains_key(k))
        .collect();
    let mut previous: Option<String> = None;
    for piece in t {
        let new = !bi.contains_key(&piece.key) && !oi.contains_key(&piece.key);
        if new && merged.contains_key(&piece.key) {
            let mut at = previous
                .as_ref()
                .and_then(|p| order.iter().position(|k| k == p))
                .map_or(0, |i| i + 1);
            // In the same spot, our new pieces come first.
            while at < order.len() && !bi.contains_key(&order[at]) && !ti.contains_key(&order[at]) {
                at += 1;
            }
            order.insert(at, piece.key.clone());
        }
        if order.contains(&piece.key) {
            previous = Some(piece.key.clone());
        }
    }
    Some(
        order
            .into_iter()
            .filter_map(|k| merged.remove(&k))
            .collect(),
    )
}

/// A piece one side kept and the other deleted: dropped if it was not
/// changed, kept if it is new, and a conflict if it was changed.
#[allow(clippy::option_option)]
fn keep_or_drop(base: Option<&str>, kept: &str) -> Option<Option<String>> {
    match base {
        Some(b) if b == kept => Some(None),
        Some(_) => None,
        None => Some(Some(kept.to_owned())),
    }
}

/// One version of the file being merged.
struct Side<'t> {
    text: &'t str,
}

impl Side<'_> {
    fn at(&self, (start, end): (usize, usize)) -> &str {
        self.text.get(start..end).unwrap_or_default()
    }
}

struct Merger<'a> {
    family: Family,
    b: Side<'a>,
    o: Side<'a>,
    t: Side<'a>,
}

impl Merger<'_> {
    fn pieces(&self, side: &Side<'_>, c: &Container<'_>) -> Vec<Piece> {
        c.parts
            .iter()
            .map(|p| Piece {
                key: p.key.clone(),
                text: side.at((p.start, p.end)).to_owned(),
            })
            .collect()
    }

    /// Whether parts inside this container may be merged by key.
    fn keyed(&self, c: &Container<'_>) -> bool {
        self.family == Family::Data
            || c.parts.iter().filter(|p| p.named).count() * 2 >= c.parts.len()
    }

    fn container(
        &self,
        b: &Container<'_>,
        o: &Container<'_>,
        t: &Container<'_>,
        depth: usize,
    ) -> Option<String> {
        // A container with one part shows no separator: it has the others'.
        let mut sep: Option<&str> = None;
        for c in [b, o, t] {
            match (&c.sep, sep) {
                (Some(s), None) => sep = Some(s),
                (Some(s), Some(seen)) if s != seen => return None,
                _ => {}
            }
        }
        if [b, o, t]
            .iter()
            .any(|c| c.sep.is_none() && c.parts.len() > 1)
            && sep.is_some()
        {
            return None;
        }
        let head = merge_text(
            Some(self.b.at(b.head)),
            self.o.at(o.head),
            self.t.at(t.head),
        )?;
        let tail = merge_text(
            Some(self.b.at(b.tail)),
            self.o.at(o.tail),
            self.t.at(t.tail),
        )?;
        let trailing = if o.trailing == b.trailing {
            t.trailing
        } else {
            o.trailing
        };
        let (pb, po, pt) = (
            self.pieces(&self.b, b),
            self.pieces(&self.o, o),
            self.pieces(&self.t, t),
        );
        let texts = merge_pieces(&pb, &po, &pt, &mut |bi, oi, ti| {
            self.nested(&b.parts[bi], &o.parts[oi], &t.parts[ti], depth + 1)
        })?;
        let mut out = head;
        let sep = sep.unwrap_or("");
        for (i, text) in texts.iter().enumerate() {
            if i > 0 {
                out.push_str(sep);
            }
            out.push_str(text);
        }
        if trailing && !texts.is_empty() {
            out.push_str(sep);
        }
        out.push_str(&tail);
        Some(out)
    }

    /// Both sides changed one part: merge what is inside it.
    fn nested(&self, b: &Part<'_>, o: &Part<'_>, t: &Part<'_>, depth: usize) -> Option<String> {
        if depth > MAX_NESTING {
            return None;
        }
        let (bb, ob, tb) = bodies(
            (b.node, self.b.text),
            (o.node, self.o.text),
            (t.node, self.t.text),
        )?;
        let (cb, co, ct) = (
            split(bb, self.b.text, false)?,
            split(ob, self.o.text, false)?,
            split(tb, self.t.text, false)?,
        );
        if !self.keyed(&cb) || !self.keyed(&co) || !self.keyed(&ct) {
            return None;
        }
        let before = |p: &Part<'_>, body: Node<'_>| (p.start, body.start_byte());
        let after = |p: &Part<'_>, body: Node<'_>| (body.end_byte(), p.end);
        let prefix = merge_text(
            Some(self.b.at(before(b, bb))),
            self.o.at(before(o, ob)),
            self.t.at(before(t, tb)),
        )?;
        let suffix = merge_text(
            Some(self.b.at(after(b, bb))),
            self.o.at(after(o, ob)),
            self.t.at(after(t, tb)),
        )?;
        let inner = self.container(&cb, &co, &ct, depth)?;
        Some(format!("{prefix}{inner}{suffix}"))
    }
}

fn is_bracket(node: Option<Node<'_>>, text: &str) -> bool {
    node.is_some_and(|n| {
        !n.is_named()
            && matches!(
                n.utf8_text(text.as_bytes()).unwrap_or_default(),
                "{" | "}" | "[" | "]" | "(" | ")"
            )
    })
}

/// Where a part keeps its own parts, as indices of named children: a
/// `body` or `value` field (a class or `impl` body, a key's value), a node
/// between brackets (an object, a list, a block), a node whose children are
/// mostly of one kind (a TOML table), or down through the only child of a
/// wrapper.
fn body_path(node: Node<'_>, text: &str, depth: usize) -> Option<Vec<usize>> {
    if depth > 6 {
        return None;
    }
    let children = named_children(node);
    if children.is_empty() {
        return None;
    }
    let mut cursor = node.walk();
    let all: Vec<Node<'_>> = node.children(&mut cursor).collect();
    if is_bracket(all.first().copied(), text) && is_bracket(all.last().copied(), text) {
        return Some(Vec::new());
    }
    for field in ["body", "value"] {
        if let Some(f) = node.child_by_field_name(field)
            && let Some(i) = children.iter().position(|c| c.id() == f.id())
        {
            let mut path = vec![i];
            path.extend(body_path(f, text, depth + 1).unwrap_or_default());
            return Some(path);
        }
    }
    if children.len() >= 2 {
        let mut count: HashMap<&str, usize> = HashMap::new();
        for c in &children {
            *count.entry(c.kind()).or_default() += 1;
        }
        if count.values().any(|&n| n * 2 >= children.len() && n >= 2) {
            return Some(Vec::new());
        }
    }
    let last = children.len() - 1;
    let mut path = vec![last];
    path.extend(body_path(children[last], text, depth + 1).unwrap_or_default());
    Some(path)
}

fn follow<'t>(node: Node<'t>, path: &[usize]) -> Option<(Node<'t>, Vec<&'t str>)> {
    let mut at = node;
    let mut kinds = Vec::with_capacity(path.len());
    for &i in path {
        at = *named_children(at).get(i)?;
        kinds.push(at.kind());
    }
    Some((at, kinds))
}

/// The body of one part in all three versions: the shortest path any of
/// them gives that leads to nodes of the same kinds in all three. A list
/// with one entry and one with two have their body in the same place.
fn bodies<'a, 'b, 'c>(
    b: (Node<'a>, &str),
    o: (Node<'b>, &str),
    t: (Node<'c>, &str),
) -> Option<(Node<'a>, Node<'b>, Node<'c>)> {
    let mut paths: Vec<Vec<usize>> = [
        body_path(b.0, b.1, 0),
        body_path(o.0, o.1, 0),
        body_path(t.0, t.1, 0),
    ]
    .into_iter()
    .flatten()
    .collect();
    paths.sort_by_key(Vec::len);
    paths.dedup();
    for path in paths {
        let (Some((nb, kb)), Some((no, ko)), Some((nt, kt))) =
            (follow(b.0, &path), follow(o.0, &path), follow(t.0, &path))
        else {
            continue;
        };
        if kb == ko && ko == kt && nb.kind() == no.kind() && no.kind() == nt.kind() {
            return Some((nb, no, nt));
        }
    }
    None
}

/// Merges `ours` and `theirs` (both changed from `base`) part by part, or
/// returns `None` if that is not possible: a part both sides changed in
/// conflicting ways, one side changed what the other deleted, or a result
/// that does not parse.
#[must_use]
pub fn merge(path: &str, base: &str, ours: &str, theirs: &str) -> Option<String> {
    if base.len().max(ours.len()).max(theirs.len()) > MAX_BYTES {
        return None;
    }
    let Some((language, family)) = grammar(path) else {
        return crate::layout::merge(path, base, ours, theirs);
    };
    // Work in progress that does not parse is left to the line merge.
    let (tb, to, tt) = (
        parse(&language, base)?,
        parse(&language, ours)?,
        parse(&language, theirs)?,
    );
    let (cb, co, ct) = (
        split(tb.root_node(), base, true)?,
        split(to.root_node(), ours, true)?,
        split(tt.root_node(), theirs, true)?,
    );
    let merger = Merger {
        family,
        b: Side { text: base },
        o: Side { text: ours },
        t: Side { text: theirs },
    };
    let out = merger.container(&cb, &co, &ct, 0)?;
    parse(&language, &out)?;
    Some(out)
}

// ------------------------------------------------------------------ search

/// A piece of a file worth finding on its own: a function, a type, a
/// method, a section. Lines are 1-based and inclusive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// The declaration's name, with its owner (`Type::method`).
    pub name: Option<String>,
    pub kind: String,
    pub start: usize,
    pub end: usize,
    pub text: String,
}

/// Lines a class or `impl` may have before its members become chunks.
const CHUNK_LINES: usize = 60;
/// Lines of a block of plain text.
const BLOCK_LINES: usize = 40;

/// Splits a file into chunks for search. Where a grammar knows the
/// language: each declaration with the comments right above it, and the
/// members of large classes, traits and modules on their own. Markdown goes
/// by heading, and any other text in blocks of about forty lines.
#[must_use]
pub fn chunks(path: &str, text: &str) -> Vec<Chunk> {
    if text.len() <= MAX_BYTES
        && let Some((language, Family::Code)) = grammar(path)
    {
        let mut parser = Parser::new();
        if parser.set_language(&language).is_ok()
            && let Some(tree) = parser.parse(text, None)
        {
            let mut out = Vec::new();
            declarations(tree.root_node(), text, None, 0, &mut out);
            if !out.is_empty() {
                return out;
            }
        }
    }
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".md") || lower.ends_with(".mdx") || lower.ends_with(".markdown") {
        return sections(text);
    }
    blocks(text, None, "text")
}

/// Whether a declaration holds members worth finding one by one.
fn holds_members(kind: &str) -> bool {
    [
        "impl",
        "class",
        "trait",
        "interface",
        "module",
        "mod_item",
        "namespace",
        "object",
        "extension",
        "protocol",
        "contract",
    ]
    .iter()
    .any(|k| kind.contains(k))
}

/// Imports and the like: noise in search results.
fn is_preamble(kind: &str) -> bool {
    [
        "import",
        "use_declaration",
        "include",
        "using",
        "package_clause",
        "extern_crate",
    ]
    .iter()
    .any(|k| kind.contains(k))
}

fn declarations(
    node: Node<'_>,
    text: &str,
    owner: Option<&str>,
    depth: usize,
    out: &mut Vec<Chunk>,
) {
    let mut cursor = node.walk();
    let children: Vec<Node<'_>> = node.named_children(&mut cursor).collect();
    // The run of comments right above the next declaration.
    let mut notes: Option<(usize, usize)> = None;
    for child in children {
        if is_comment(&child) {
            let row = child.start_position().row;
            notes = match notes {
                Some((start, end)) if row <= end + 1 => Some((start, child.end_position().row)),
                _ => Some((child.start_byte(), child.end_position().row)),
            };
            continue;
        }
        let above = notes
            .take()
            .filter(|(_, end)| child.start_position().row <= end + 1)
            .map(|(start, _)| start);
        let kind = child.kind();
        if is_preamble(kind) {
            continue;
        }
        let name = name_of(child, text);
        let qualified = match (owner, &name) {
            (Some(o), Some(n)) => Some(format!("{o}::{n}")),
            (None, n) => n.clone(),
            (Some(_), None) => None,
        };
        let first = child.start_position().row;
        let last = child.end_position().row;
        if name.is_none() && last - first < 2 {
            continue;
        }
        let start = above.unwrap_or_else(|| child.start_byte());
        let start_line = text[..start].matches('\n').count() + 1;
        if depth < 2
            && last - first + 1 > CHUNK_LINES
            && holds_members(kind)
            && let Some(path) = body_path(child, text, 0)
            && let Some((body, _)) = follow(child, &path)
        {
            let before = out.len();
            declarations(body, text, qualified.as_deref(), depth + 1, out);
            if out.len() > before {
                // The declaration itself: its comments and its first line.
                let head_end = body.start_byte().max(start);
                out.insert(
                    before,
                    Chunk {
                        name: qualified,
                        kind: kind.to_owned(),
                        start: start_line,
                        end: body.start_position().row + 1,
                        text: text[start..head_end].to_owned(),
                    },
                );
                continue;
            }
        }
        out.push(Chunk {
            name: qualified,
            kind: kind.to_owned(),
            start: start_line,
            end: last + 1,
            text: text[start..child.end_byte()].to_owned(),
        });
    }
}

/// Markdown by heading, outside code fences.
fn sections(text: &str) -> Vec<Chunk> {
    let mut out = Vec::new();
    let mut fence = false;
    let mut current: Option<(String, usize, String)> = None;
    let mut lead = String::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            fence = !fence;
        }
        if !fence && line.starts_with('#') {
            if let Some((name, start, body)) = current.take() {
                out.push(Chunk {
                    name: Some(name),
                    kind: "section".to_owned(),
                    start,
                    end: i,
                    text: body,
                });
            } else if !lead.trim().is_empty() {
                out.extend(blocks(&lead, None, "text"));
            }
            let name = line.trim_start_matches('#').trim().to_owned();
            current = Some((name, i + 1, format!("{line}\n")));
            continue;
        }
        match &mut current {
            Some((_, _, body)) => {
                body.push_str(line);
                body.push('\n');
            }
            None => {
                lead.push_str(line);
                lead.push('\n');
            }
        }
    }
    let total = text.lines().count();
    match current {
        Some((name, start, body)) => {
            out.push(Chunk {
                name: Some(name),
                kind: "section".to_owned(),
                start,
                end: total,
                text: body,
            });
        }
        None => out.extend(blocks(&lead, None, "text")),
    }
    out
}

/// Plain text in blocks that end at a blank line once they are long enough.
fn blocks(text: &str, name: Option<&str>, kind: &str) -> Vec<Chunk> {
    let mut out = Vec::new();
    let mut start = 1;
    let mut body = String::new();
    let mut lines = 0;
    for (i, line) in text.lines().enumerate() {
        body.push_str(line);
        body.push('\n');
        lines += 1;
        if (lines >= BLOCK_LINES && line.trim().is_empty()) || lines >= BLOCK_LINES * 2 {
            if !body.trim().is_empty() {
                out.push(Chunk {
                    name: name.map(str::to_owned),
                    kind: kind.to_owned(),
                    start,
                    end: i + 1,
                    text: std::mem::take(&mut body),
                });
            }
            body.clear();
            start = i + 2;
            lines = 0;
        }
    }
    if !body.trim().is_empty() {
        out.push(Chunk {
            name: name.map(str::to_owned),
            kind: kind.to_owned(),
            start,
            end: start + lines - 1,
            text: body,
        });
    }
    out
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
    fn real_conflicts_are_left_alone() {
        let ours = "fn one() -> u8 {\n    10\n}\n";
        let theirs = "fn one() -> u8 {\n    20\n}\n";
        assert!(merge("lib.rs", BASE, ours, theirs).is_none());
        // Edited on one side, deleted on the other.
        let edited = BASE.replace("    1", "    100");
        assert!(merge("lib.rs", BASE, &edited, "use std::fs;\n").is_none());
        // Two edits inside one function body: statements keep their order.
        let base = "fn f() {\n    a();\n    b();\n}\n";
        let ours = "fn f() {\n    a();\n    x();\n    b();\n}\n";
        let theirs = "fn f() {\n    a();\n    y();\n    b();\n}\n";
        assert!(merge("lib.rs", base, ours, theirs).is_none());
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

    #[test]
    fn methods_added_to_one_impl_block_merge() {
        let base = "struct S;\n\nimpl S {\n    fn a(&self) {}\n}\n";
        let ours = "struct S;\n\nimpl S {\n    fn a(&self) {}\n\n    fn b(&self) {}\n}\n";
        let theirs = "struct S;\n\nimpl S {\n    fn a(&self) {}\n\n    fn c(&self) {}\n}\n";
        let merged = merge("s.rs", base, ours, theirs).unwrap();
        assert_eq!(
            merged,
            "struct S;\n\nimpl S {\n    fn a(&self) {}\n\n    fn b(&self) {}\n\n    fn c(&self) {}\n}\n"
        );
        // Same for a TypeScript class and a Java class.
        let base = "class A {\n  a() {}\n}\n";
        let ours = "class A {\n  a() {}\n  b() {}\n}\n";
        let theirs = "class A {\n  a() {}\n  c() {}\n}\n";
        let merged = merge("a.ts", base, ours, theirs).unwrap();
        assert!(
            merged.contains("b() {}") && merged.contains("c() {}"),
            "{merged}"
        );
        let java = |m: &str| format!("class A {{\n    void a() {{}}\n{m}}}\n");
        let merged = merge(
            "A.java",
            &java(""),
            &java("    void b() {}\n"),
            &java("    void c() {}\n"),
        )
        .unwrap();
        assert!(
            merged.contains("void b()") && merged.contains("void c()"),
            "{merged}"
        );
    }

    #[test]
    fn json_keys_merge_with_their_commas() {
        let base = "{\n  \"name\": \"app\",\n  \"dependencies\": {\n    \"a\": \"1.0.0\"\n  }\n}\n";
        let ours = "{\n  \"name\": \"app\",\n  \"dependencies\": {\n    \"a\": \"1.0.0\",\n    \"b\": \"2.0.0\"\n  }\n}\n";
        let theirs = "{\n  \"name\": \"app\",\n  \"dependencies\": {\n    \"a\": \"1.0.0\",\n    \"c\": \"3.0.0\"\n  }\n}\n";
        let merged = merge("package.json", base, ours, theirs).unwrap();
        assert_eq!(
            merged,
            "{\n  \"name\": \"app\",\n  \"dependencies\": {\n    \"a\": \"1.0.0\",\n    \"b\": \"2.0.0\",\n    \"c\": \"3.0.0\"\n  }\n}\n"
        );
        // The same key set to two values is a conflict.
        let one = base.replace("1.0.0", "1.1.0");
        let two = base.replace("1.0.0", "1.2.0");
        assert!(merge("package.json", base, &one, &two).is_none());
    }

    #[test]
    fn toml_tables_and_lockfiles_merge() {
        let base = "[package]\nname = \"x\"\n\n[dependencies]\nserde = \"1\"\n";
        let ours = "[package]\nname = \"x\"\n\n[dependencies]\nserde = \"1\"\nanyhow = \"1\"\n";
        let theirs = "[package]\nname = \"x\"\n\n[dependencies]\nserde = \"1\"\nclap = \"4\"\n";
        let merged = merge("Cargo.toml", base, ours, theirs).unwrap();
        assert!(
            merged.contains("anyhow = \"1\"\n") && merged.contains("clap = \"4\"\n"),
            "{merged}"
        );

        let pkg = |n: &str| format!("[[package]]\nname = \"{n}\"\nversion = \"1.0.0\"\n");
        let base = format!("version = 4\n\n{}\n{}", pkg("a"), pkg("m"));
        let ours = format!("version = 4\n\n{}\n{}\n{}", pkg("a"), pkg("b"), pkg("m"));
        let theirs = format!("version = 4\n\n{}\n{}\n{}", pkg("a"), pkg("m"), pkg("z"));
        let merged = merge("Cargo.lock", &base, &ours, &theirs).unwrap();
        for n in ["a", "b", "m", "z"] {
            assert!(merged.contains(&format!("name = \"{n}\"")), "{merged}");
        }
    }

    #[test]
    fn yaml_mappings_merge_inside() {
        let base = "jobs:\n  test:\n    runs-on: ubuntu\n";
        let ours = "jobs:\n  test:\n    runs-on: ubuntu\n  lint:\n    runs-on: ubuntu\n";
        let theirs = "jobs:\n  test:\n    runs-on: ubuntu\n  build:\n    runs-on: ubuntu\n";
        let merged = merge("ci.yml", base, ours, theirs).unwrap();
        assert!(
            merged.contains("  lint:") && merged.contains("  build:"),
            "{merged}"
        );
    }

    /// Each side adds one declaration after the last: merges in each language.
    #[test]
    fn additions_merge_across_languages() {
        let cases: [(&str, &str, &str, &str); 9] = [
            (
                "a.py",
                "class A:\n    def a(self):\n        return 1\n",
                "    def b(self):\n        return 2\n",
                "    def c(self):\n        return 3\n",
            ),
            ("a.go", "package a\n\ntype S struct {\n\tA int\n}\n", "", ""),
            (
                "a.c",
                "int a(void) {\n    return 1;\n}\n",
                "\nint b(void) {\n    return 2;\n}\n",
                "\nint c(void) {\n    return 3;\n}\n",
            ),
            (
                "a.kt",
                "fun a() = 1\n",
                "\nfun b() = 2\n",
                "\nfun c() = 3\n",
            ),
            (
                "a.swift",
                "func a() -> Int { 1 }\n",
                "\nfunc b() -> Int { 2 }\n",
                "\nfunc c() -> Int { 3 }\n",
            ),
            (
                "a.rb",
                "def a\n  1\nend\n",
                "\ndef b\n  2\nend\n",
                "\ndef c\n  3\nend\n",
            ),
            ("a.cs", "class A {\n    void X() {}\n}\n", "", ""),
            (
                "a.php",
                "<?php\nfunction a() { return 1; }\n",
                "function b() { return 2; }\n",
                "function c() { return 3; }\n",
            ),
            (
                "a.lua",
                "function a() return 1 end\n",
                "\nfunction b() return 2 end\n",
                "\nfunction c() return 3 end\n",
            ),
        ];
        for (path, base, add_o, add_t) in cases {
            if add_o.is_empty() {
                continue;
            }
            let (ours, theirs) = (format!("{base}{add_o}"), format!("{base}{add_t}"));
            let merged =
                merge(path, base, &ours, &theirs).unwrap_or_else(|| panic!("{path} did not merge"));
            assert!(
                merged.contains(add_o.trim()) && merged.contains(add_t.trim()),
                "{path}: {merged}"
            );
        }
        // Fields added to one Go struct, members to one C# class.
        let go = |f: &str| format!("package a\n\ntype S struct {{\n\tA int\n{f}}}\n");
        let merged = merge("a.go", &go(""), &go("\tB int\n"), &go("\tC int\n")).unwrap();
        assert!(
            merged.contains("\tB int") && merged.contains("\tC int"),
            "{merged}"
        );
        let cs = |m: &str| format!("class A {{\n    void X() {{}}\n{m}}}\n");
        let merged = merge(
            "a.cs",
            &cs(""),
            &cs("    void Y() {}\n"),
            &cs("    void Z() {}\n"),
        )
        .unwrap();
        assert!(
            merged.contains("void Y()") && merged.contains("void Z()"),
            "{merged}"
        );
    }

    #[test]
    fn the_same_name_changed_twice_stays_a_conflict() {
        let base = "fn old() {}\n";
        assert!(merge("a.rs", base, "fn mine() {}\n", "fn yours() {}\n").is_none());
        let base = "{\"a\": 1, \"b\": 2}\n";
        assert!(
            merge(
                "a.json",
                base,
                "{\"a\": 3, \"b\": 2}\n",
                "{\"a\": 4, \"b\": 2}\n"
            )
            .is_none()
        );
    }

    #[test]
    fn every_grammar_loads_and_parses() {
        let samples = [
            ("a.c", "int f(void) { return 1; }\n"),
            #[cfg(feature = "extra-grammars")]
            ("a.cpp", "class A { int f(); };\n"),
            #[cfg(feature = "extra-grammars")]
            ("a.cs", "class A { void F() {} }\n"),
            ("a.rb", "def f\n  1\nend\n"),
            ("a.php", "<?php\nfunction f() { return 1; }\n"),
            ("a.sh", "f() { echo hi; }\n"),
            ("a.json", "{\"a\": 1}\n"),
            ("a.css", "a { color: red; }\n"),
            ("a.html", "<p>hi</p>\n"),
            #[cfg(feature = "extra-grammars")]
            ("a.scala", "object A { def f = 1 }\n"),
            ("a.lua", "function f() return 1 end\n"),
            #[cfg(feature = "extra-grammars")]
            ("a.kt", "fun f() = 1\n"),
            #[cfg(feature = "extra-grammars")]
            ("a.swift", "func f() -> Int { return 1 }\n"),
            #[cfg(feature = "extra-grammars")]
            ("a.hs", "f :: Int\nf = 1\n"),
            #[cfg(feature = "extra-grammars")]
            ("a.ex", "defmodule A do\nend\n"),
            #[cfg(feature = "extra-grammars")]
            ("a.zig", "fn f() void {}\n"),
            ("a.yaml", "a: 1\n"),
            ("a.toml", "a = 1\n"),
            ("a.tf", "variable \"a\" {}\n"),
            #[cfg(feature = "extra-grammars")]
            ("a.ml", "let f x = x\n"),
            #[cfg(feature = "extra-grammars")]
            ("a.dart", "int f() => 1;\n"),
            #[cfg(feature = "extra-grammars")]
            ("a.sol", "contract A {}\n"),
            ("a.proto", "syntax = \"proto3\";\nmessage A {}\n"),
            #[cfg(feature = "extra-grammars")]
            ("a.r", "f <- function() 1\n"),
            #[cfg(feature = "extra-grammars")]
            ("a.jl", "f(x) = x\n"),
            ("a.nix", "{ a = 1; }\n"),
            #[cfg(feature = "extra-grammars")]
            ("a.m", "int f(void) { return 1; }\n"),
            #[cfg(feature = "extra-grammars")]
            ("a.ps1", "function F { 1 }\n"),
            #[cfg(feature = "extra-grammars")]
            ("a.elm", "module A exposing (..)\n\nf = 1\n"),
            #[cfg(feature = "extra-grammars")]
            ("a.erl", "-module(a).\nf() -> 1.\n"),
            #[cfg(feature = "extra-grammars")]
            ("a.gleam", "pub fn f() { 1 }\n"),
        ];
        for (path, text) in samples {
            let (language, _) = grammar(path).unwrap();
            assert!(parse(&language, text).is_some(), "{path} does not parse");
        }
    }

    #[test]
    fn chunks_follow_declarations_and_their_comments() {
        let mut text = String::from(
            "use std::fmt;\n\n/// Adds.\nfn add(a: u32, b: u32) -> u32 {\n    a + b\n}\n\nstruct Point { x: i32 }\n\n",
        );
        text.push_str("/// A big one.\nimpl Point {\n");
        for i in 0..40 {
            text.push_str(&format!(
                "    fn m{i}(&self) {{\n        let _ = {i};\n    }}\n"
            ));
        }
        text.push_str("}\n");
        let got = chunks("a.rs", &text);
        let names: Vec<Option<&str>> = got.iter().map(|c| c.name.as_deref()).collect();
        assert_eq!(names[0], Some("add"));
        assert!(got[0].text.starts_with("/// Adds."), "{:?}", got[0]);
        assert_eq!((got[0].start, got[0].end), (3, 6));
        assert_eq!(names[1], Some("Point"));
        assert_eq!(names[2], Some("Point"), "the impl's own line");
        assert!(names.contains(&Some("Point::m7")), "{names:?}");
        assert!(!got.iter().any(|c| c.text.contains("use std")));

        let md = chunks(
            "README.md",
            "Intro.\n\n# Install\nRun it.\n\n## Use\n```\n# not a heading\n```\n",
        );
        let titles: Vec<Option<&str>> = md.iter().map(|c| c.name.as_deref()).collect();
        assert_eq!(titles, [None, Some("Install"), Some("Use")]);
        assert_eq!(chunks("notes.txt", "a\nb\n").len(), 1);
    }
}
