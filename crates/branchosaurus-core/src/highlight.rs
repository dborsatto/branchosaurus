// SPDX-License-Identifier: GPL-3.0-or-later

//! Syntax highlighting for diffs.
//!
//! The core decides what each piece of text is (a keyword, a string, a comment…); each app picks
//! the colors, so they follow the platform's look and its light and dark appearances. Whole files
//! are highlighted, not just the lines a diff shows, so a comment or string that starts above a
//! hunk is still recognised inside it.
//!
//! Grammars are syntect's Sublime Text syntaxes plus bat's extra set, via the `two-face` crate.

use std::sync::OnceLock;

use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet};

use crate::RepoPath;

/// Diffs that need more parsing than this are shown plain. Grammars parse about 1.5 MB/s on an
/// M-series Mac, so this keeps highlighting under ~200 ms; it covers the largest hand-written
/// files of most projects, and skips generated ones.
pub(crate) const MAX_HIGHLIGHT_BYTES: usize = 256 * 1024;
/// Lines longer than this (minified code, data) make grammar matching slow; files containing
/// them are shown without highlighting.
const MAX_HIGHLIGHT_LINE: usize = 4_000;

/// What a piece of code is. Apps map these to colors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HighlightKind {
    Keyword,
    String,
    Comment,
    Number,
    /// Other literal constants: `true`, `nil`, character escapes…
    Constant,
    Type,
    Function,
    /// Annotations and attributes (`#[derive]`, `@objc`) and attribute names in markup.
    Attribute,
    /// Markup tags.
    Tag,
    /// Fields and properties.
    Property,
    Operator,
}

/// A highlighted range of a line, in UTF-8 byte offsets into the line's text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Highlight {
    pub start: u32,
    pub end: u32,
    pub kind: HighlightKind,
}

/// A language the highlighter knows.
#[derive(Clone, Copy)]
pub(crate) struct Language {
    syntax: &'static SyntaxReference,
}

impl std::fmt::Debug for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.syntax.name)
    }
}

impl Language {
    pub fn name(&self) -> &'static str {
        &self.syntax.name
    }
}

fn syntaxes() -> &'static SyntaxSet {
    static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
    SYNTAXES.get_or_init(two_face::syntax::extra_newlines)
}

/// The licenses of the bundled syntax definitions that ask to be acknowledged, as Markdown, for
/// shipping with the apps.
pub fn syntax_acknowledgements() -> String {
    let mut markdown = String::from(
        "# Syntax definitions\n\n\
         Branchosaurus highlights code with the syntax definitions collected by the \
         [bat](https://github.com/sharkdp/bat) project and packaged by \
         [two-face](https://codeberg.org/CosmicHarper/two-face). Their licenses follow.\n\n",
    );
    for license in two_face::acknowledgement::listing().for_syntaxes() {
        license.write_md(&mut markdown);
    }
    markdown
}

/// Picks a language from the file's name (`Makefile`, `main.rs`) or, failing that, its first line
/// (`#!/usr/bin/env python3`, `<?xml`).
pub(crate) fn detect(path: &RepoPath, first_line: &[u8]) -> Option<Language> {
    let syntaxes = syntaxes();
    let name = String::from_utf8_lossy(path.file_name()).into_owned();
    let by_name = syntaxes.find_syntax_by_extension(&name);
    let by_extension = || {
        let (_, extension) = name.rsplit_once('.')?;
        syntaxes.find_syntax_by_extension(extension)
    };
    let by_first_line = || {
        let line = String::from_utf8_lossy(first_line);
        syntaxes.find_syntax_by_first_line(&line)
    };
    let syntax = by_name.or_else(by_extension).or_else(by_first_line)?;
    // "Plain Text" is a valid syntax, but highlighting it is pointless.
    (syntax.name != "Plain Text").then_some(Language { syntax })
}

/// Highlights a file line by line, from its first line. Two highlighters that have read the
/// same lines compare equal, so diffs can parse the lines both versions share once.
#[derive(Clone)]
pub(crate) struct LineHighlighter {
    state: ParseState,
    stack: ScopeStack,
}

impl PartialEq for LineHighlighter {
    fn eq(&self, other: &Self) -> bool {
        self.stack == other.stack && self.state == other.state
    }
}

impl LineHighlighter {
    pub fn new(language: Language) -> Self {
        Self {
            state: ParseState::new(language.syntax),
            stack: ScopeStack::new(),
        }
    }

    /// Reads the next line (its text without a terminator). Returns its highlights, or `None`
    /// when the line is too long or the grammar fails; the highlighter can't continue then.
    /// With `emit` false the line only advances the state, which is cheaper.
    pub fn line(&mut self, line: &str, emit: bool) -> Option<Vec<Highlight>> {
        if line.len() > MAX_HIGHLIGHT_LINE {
            return None;
        }
        // The "newlines" grammars expect each line to end in a newline.
        let mut buffer = String::with_capacity(line.len() + 1);
        buffer.push_str(line);
        buffer.push('\n');
        let operations = self.state.parse_line(&buffer, syntaxes()).ok()?;
        if !emit {
            for (_, operation) in &operations {
                self.stack.apply(operation).ok()?;
            }
            return Some(Vec::new());
        }
        let rules = rules();
        let mut highlights: Vec<Highlight> = Vec::new();
        let mut push = |start: usize, end: usize, stack: &ScopeStack| {
            let end = end.min(line.len());
            if start >= end {
                return;
            }
            let Some(kind) = classify(stack, rules) else {
                return;
            };
            match highlights.last_mut() {
                Some(last) if last.kind == kind && last.end as usize == start => {
                    last.end = end as u32
                }
                _ => highlights.push(Highlight {
                    start: start as u32,
                    end: end as u32,
                    kind,
                }),
            }
        };
        let mut position = 0;
        for (offset, operation) in &operations {
            push(position, *offset, &self.stack);
            self.stack.apply(operation).ok()?;
            position = *offset;
        }
        push(position, line.len(), &self.stack);
        Some(highlights)
    }
}

/// The kind of the innermost scope that has one. Punctuation scopes have none, so the quotes of a
/// string are colored as the string and the `//` of a comment as the comment.
fn classify(stack: &ScopeStack, rules: &[(Scope, HighlightKind)]) -> Option<HighlightKind> {
    stack.as_slice().iter().rev().find_map(|scope| {
        rules
            .iter()
            .find(|(prefix, _)| prefix.is_prefix_of(*scope))
            .map(|(_, kind)| *kind)
    })
}

/// TextMate scope names to kinds. More specific prefixes come first.
fn rules() -> &'static [(Scope, HighlightKind)] {
    static RULES: OnceLock<Vec<(Scope, HighlightKind)>> = OnceLock::new();
    RULES.get_or_init(|| {
        use HighlightKind::*;
        [
            ("comment", Comment),
            ("string", String),
            ("markup.raw", String),
            ("constant.numeric", Number),
            ("constant.character.escape", Constant),
            ("constant", Constant),
            ("keyword.operator", Operator),
            ("keyword", Keyword),
            ("storage", Keyword),
            ("markup.heading", Keyword),
            ("entity.name.function", Function),
            ("support.function", Function),
            ("variable.function", Function),
            ("support.macro", Function),
            ("entity.name.tag", Tag),
            ("entity.other.attribute-name", Attribute),
            ("meta.annotation", Attribute),
            ("meta.attribute", Attribute),
            ("entity.other.inherited-class", Type),
            ("entity.name", Type),
            ("support.type", Type),
            ("support.class", Type),
            ("variable.other.member", Property),
            ("variable.other.property", Property),
            ("support.variable.property", Property),
            ("variable.language", Constant),
        ]
        .into_iter()
        .filter_map(|(scope, kind)| Some((Scope::new(scope).ok()?, kind)))
        .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn highlight_lines(language: Language, lines: &[&str]) -> Option<Vec<Vec<Highlight>>> {
        let mut highlighter = LineHighlighter::new(language);
        lines
            .iter()
            .map(|line| highlighter.line(line, true))
            .collect()
    }

    fn kinds(path: &str, code: &str) -> Vec<Vec<(String, HighlightKind)>> {
        let language = detect(
            &RepoPath::from(path),
            code.lines().next().unwrap_or("").as_bytes(),
        )
        .unwrap_or_else(|| panic!("no language for {path}"));
        let lines: Vec<&str> = code.lines().collect();
        let highlights = highlight_lines(language, &lines).expect("highlighted");
        lines
            .iter()
            .zip(highlights)
            .map(|(line, highlights)| {
                highlights
                    .iter()
                    .map(|h| (line[h.start as usize..h.end as usize].to_owned(), h.kind))
                    .collect()
            })
            .collect()
    }

    fn has(line: &[(String, HighlightKind)], text: &str, kind: HighlightKind) -> bool {
        line.iter().any(|(t, k)| t.contains(text) && *k == kind)
    }

    #[test]
    fn detects_languages_by_name_extension_and_first_line() {
        let name = |path: &str, first: &str| {
            detect(&RepoPath::from(path), first.as_bytes()).map(|l| l.name())
        };
        assert_eq!(name("src/main.rs", ""), Some("Rust"));
        assert_eq!(name("apps/macos/Package.swift", ""), Some("Swift"));
        assert_eq!(name("web/index.ts", ""), Some("TypeScript"));
        assert_eq!(name("Cargo.toml", ""), Some("TOML"));
        assert_eq!(name("Makefile", ""), Some("Makefile"));
        assert_eq!(name("docker/Dockerfile", ""), Some("Dockerfile"));
        assert_eq!(name("bin/tool", "#!/usr/bin/env python3"), Some("Python"));
        assert_eq!(name("notes.unknownext", "hello"), None);
        assert_eq!(name("notes.txt", "hello"), None);
    }

    #[test]
    fn highlights_rust() {
        let lines = kinds(
            "lib.rs",
            "// A comment\nfn answer() -> u32 {\n    let text = \"forty-two\";\n    42\n}",
        );
        assert!(
            has(&lines[0], "// A comment", HighlightKind::Comment),
            "{:?}",
            lines[0]
        );
        assert!(
            has(&lines[1], "fn", HighlightKind::Keyword),
            "{:?}",
            lines[1]
        );
        assert!(
            has(&lines[1], "answer", HighlightKind::Function),
            "{:?}",
            lines[1]
        );
        assert!(
            has(&lines[1], "u32", HighlightKind::Type)
                || has(&lines[1], "u32", HighlightKind::Keyword),
            "{:?}",
            lines[1]
        );
        assert!(
            has(&lines[2], "let", HighlightKind::Keyword),
            "{:?}",
            lines[2]
        );
        // The quotes belong to the string.
        assert!(
            has(&lines[2], "\"forty-two\"", HighlightKind::String),
            "{:?}",
            lines[2]
        );
        assert!(
            has(&lines[3], "42", HighlightKind::Number),
            "{:?}",
            lines[3]
        );
    }

    #[test]
    fn state_carries_across_lines() {
        let lines = kinds(
            "styles.c",
            "int a; /* starts here\n   still a comment\n */ int b;",
        );
        assert!(
            has(&lines[1], "still a comment", HighlightKind::Comment),
            "{:?}",
            lines[1]
        );
        assert!(
            has(&lines[2], "int", HighlightKind::Keyword),
            "{:?}",
            lines[2]
        );
    }

    #[test]
    fn offsets_are_utf8_and_within_lines() {
        let code = "let café = \"naïve ☕\"; // ünïcode";
        let language = detect(&RepoPath::from("x.rs"), b"").unwrap();
        let highlights = highlight_lines(language, &[code]).unwrap();
        for highlight in &highlights[0] {
            assert!(highlight.start < highlight.end);
            assert!(code.is_char_boundary(highlight.start as usize));
            assert!(code.is_char_boundary(highlight.end as usize));
        }
        let comment = highlights[0]
            .iter()
            .find(|h| h.kind == HighlightKind::Comment)
            .unwrap();
        assert_eq!(
            &code[comment.start as usize..comment.end as usize],
            "// ünïcode"
        );
    }

    #[test]
    fn skips_minified_lines() {
        let language = detect(&RepoPath::from("x.js"), b"").unwrap();
        let long = "a".repeat(MAX_HIGHLIGHT_LINE + 1);
        assert!(highlight_lines(language, &[&long]).is_none());
        assert!(highlight_lines(language, &["const x = 1;"]).is_some());
    }

    #[test]
    fn acknowledges_syntax_licenses() {
        let markdown = syntax_acknowledgements();
        assert!(markdown.starts_with("# Syntax definitions"));
        assert!(markdown.matches("<summary>").count() > 10, "{markdown}");
    }

    #[test]
    fn highlighters_compare_by_state() {
        let language = detect(&RepoPath::from("x.c"), b"").unwrap();
        let mut a = LineHighlighter::new(language);
        let mut b = LineHighlighter::new(language);
        a.line("int a;", false).unwrap();
        b.line("int b;", false).unwrap();
        assert!(a == b, "same state after different plain lines");
        b.line("/* open", false).unwrap();
        a.line("int c;", false).unwrap();
        assert!(a != b, "one is inside a comment");
        b.line("close */", false).unwrap();
        a.line("int d;", false).unwrap();
        assert!(a == b, "back in sync after the comment closes");
    }
}
