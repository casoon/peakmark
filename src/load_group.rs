//! What a page loads next to the module: the JavaScript glue and the files it
//! reaches by static relative specifiers.
//!
//! The glue is read as text and never executed. A small scanner finds
//! `import ... from "./x"`, `import "./x"`, `export ... from "./x"` and
//! `new URL("./x", import.meta.url)`; everything it cannot follow with
//! certainty is reported as not followed, never silently dropped.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;

use crate::compress::{brotli_len, gzip_len};
use crate::error::Error;

/// How a file is referenced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// `import` / `export ... from`: a module specifier, relative only with `./` or `../`.
    Module,
    /// `new URL(x, import.meta.url)`: relative to the file unless it has a scheme or a leading `/`.
    Url,
}

/// A reference found in the text of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Reference {
    /// A string literal that names a file.
    Static(String, Kind),
    /// A dynamic `import()`, or a `new URL(..., import.meta.url)` that is not a literal. The
    /// specifier is set if it is a literal inside `import("...")`.
    Dynamic(Option<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Ident(String),
    Str(String),
    Punct(char),
    Other,
}

/// Splits `text` into the tokens the scanner needs. Comments, template
/// literal text and regular expression literals are skipped; strings keep
/// their raw contents.
fn tokenize(text: &str) -> Vec<Token> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut templates: Vec<usize> = Vec::new();
    let mut depth = 0usize;
    let mut i = 0;
    // Reads template text from `i` up to the closing backtick or a `${`.
    let template = |mut i: usize, templates: &mut Vec<usize>, depth: usize| {
        while i < chars.len() {
            match chars[i] {
                '\\' => i += 2,
                '`' => return i + 1,
                '$' if chars.get(i + 1) == Some(&'{') => {
                    templates.push(depth);
                    return i + 2;
                }
                _ => i += 1,
            }
        }
        i
    };
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i += 2;
        } else if c == '"' || c == '\'' {
            let start = i + 1;
            i += 1;
            while i < chars.len() && chars[i] != c && chars[i] != '\n' {
                i += if chars[i] == '\\' { 2 } else { 1 };
            }
            tokens.push(Token::Str(
                chars[start..i.min(chars.len())].iter().collect(),
            ));
            i += 1;
        } else if c == '`' {
            i = template(i + 1, &mut templates, depth);
            tokens.push(Token::Other);
        } else if c == '{' {
            depth += 1;
            tokens.push(Token::Punct('{'));
            i += 1;
        } else if c == '}' {
            if templates.last() == Some(&depth) {
                templates.pop();
                i = template(i + 1, &mut templates, depth);
                tokens.push(Token::Other);
            } else {
                depth = depth.saturating_sub(1);
                tokens.push(Token::Punct('}'));
                i += 1;
            }
        } else if c == '/' && regex_allowed(tokens.last()) {
            // A regular expression literal: skip it so its quotes are not read as strings.
            i += 1;
            let mut class = false;
            while i < chars.len() && chars[i] != '\n' && (class || chars[i] != '/') {
                match chars[i] {
                    '\\' => i += 1,
                    '[' => class = true,
                    ']' => class = false,
                    _ => {}
                }
                i += 1;
            }
            i += 1;
            while i < chars.len() && chars[i].is_alphabetic() {
                i += 1;
            }
            tokens.push(Token::Other);
        } else if c.is_alphabetic() || c == '_' || c == '$' {
            let start = i;
            while i < chars.len()
                && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '$')
            {
                i += 1;
            }
            tokens.push(Token::Ident(chars[start..i].iter().collect()));
        } else if c.is_ascii_digit() {
            while i < chars.len()
                && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '.')
            {
                i += 1;
            }
            tokens.push(Token::Other);
        } else {
            tokens.push(Token::Punct(c));
            i += 1;
        }
    }
    tokens
}

/// Whether a `/` after `previous` starts a regular expression rather than a division.
fn regex_allowed(previous: Option<&Token>) -> bool {
    match previous {
        None => true,
        Some(Token::Punct(c)) => !matches!(c, ')' | ']' | '}'),
        Some(Token::Ident(word)) => matches!(
            word.as_str(),
            "return" | "typeof" | "case" | "in" | "of" | "delete" | "void" | "throw" | "else"
        ),
        Some(Token::Str(_) | Token::Other) => false,
    }
}

fn is_ident(token: Option<&Token>, word: &str) -> bool {
    matches!(token, Some(Token::Ident(w)) if w == word)
}

fn is_punct(token: Option<&Token>, c: char) -> bool {
    token == Some(&Token::Punct(c))
}

/// Whether `tokens` starts with `import . meta . url`.
fn is_import_meta_url(tokens: &[Token]) -> bool {
    is_ident(tokens.first(), "import")
        && is_punct(tokens.get(1), '.')
        && is_ident(tokens.get(2), "meta")
        && is_punct(tokens.get(3), '.')
        && is_ident(tokens.get(4), "url")
}

/// The references in `text`, in the order they appear.
fn scan(text: &str) -> Vec<Reference> {
    let tokens = tokenize(text);
    let mut references = Vec::new();
    for (at, token) in tokens.iter().enumerate() {
        let Token::Ident(word) = token else { continue };
        let after = &tokens[at + 1..];
        match word.as_str() {
            // `a.import(...)` and `a.export` are properties, not statements.
            "import" | "export" if at > 0 && is_punct(tokens.get(at - 1), '.') => {}
            "import" if is_punct(after.first(), '.') => {}
            "import" if is_punct(after.first(), '(') => {
                references.push(Reference::Dynamic(match (after.get(1), after.get(2)) {
                    (Some(Token::Str(s)), Some(Token::Punct(')' | ','))) => Some(s.clone()),
                    _ => None,
                }));
            }
            "import" => {
                if let Some(Token::Str(s)) = after.first() {
                    references.push(Reference::Static(s.clone(), Kind::Module));
                } else if let Some(s) = from_clause(after) {
                    references.push(Reference::Static(s, Kind::Module));
                }
            }
            "export" if is_punct(after.first(), '{') || is_punct(after.first(), '*') => {
                if let Some(s) = from_clause(after) {
                    references.push(Reference::Static(s, Kind::Module));
                }
            }
            "new" if is_ident(after.first(), "URL") && is_punct(after.get(1), '(') => {
                match (after.get(2), after.get(3), after.get(4..)) {
                    (Some(Token::Str(s)), Some(Token::Punct(',')), Some(rest))
                        if is_import_meta_url(rest) =>
                    {
                        references.push(Reference::Static(s.clone(), Kind::Url));
                    }
                    _ => {
                        let window = &after[2..after.len().min(40)];
                        if (0..window.len()).any(|k| is_import_meta_url(&window[k..])) {
                            references.push(Reference::Dynamic(None));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    references
}

/// The string after `from` in an import or export clause starting at `tokens`,
/// if the clause ends in one before the next statement starts.
fn from_clause(tokens: &[Token]) -> Option<String> {
    for (at, token) in tokens.iter().enumerate() {
        match token {
            Token::Punct(';') => return None,
            Token::Ident(w) if matches!(w.as_str(), "import" | "export") => return None,
            Token::Str(s) if at > 0 && is_ident(tokens.get(at - 1), "from") => {
                return Some(s.clone());
            }
            _ => {}
        }
    }
    None
}

/// One file of the group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LoadFile {
    /// The path relative to the glue's directory, with `/`.
    pub path: String,
    /// Size on disk.
    pub raw: u64,
    /// Size compressed with gzip on its own.
    pub gzip: u64,
    /// Size compressed with brotli on its own.
    pub brotli: u64,
}

/// Why a reference was not followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// A dynamic `import()` or a computed `new URL(..., import.meta.url)`.
    Dynamic,
    /// The file does not exist.
    Missing,
    /// The file is outside the glue's directory.
    OutsideDirectory,
    /// A bare module name, an absolute path or a URL: not a file next to the glue.
    NotRelative,
}

/// A reference that is not part of the group, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NotFollowed {
    /// The file that holds the reference, relative to the glue's directory.
    pub from: String,
    /// The specifier as written; `None` if it is computed.
    pub specifier: Option<String>,
    /// Why it was not followed.
    pub reason: Reason,
}

/// The glue and what it reaches, measured file by file. Observed, never gated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LoadGroup {
    /// The glue first, then the files in the order they were found.
    pub files: Vec<LoadFile>,
    /// Sum of the files' sizes on disk.
    pub raw: u64,
    /// Sum of the files' gzip sizes, each compressed on its own.
    pub gzip: u64,
    /// Sum of the files' brotli sizes, each compressed on its own.
    pub brotli: u64,
    /// What the scan could not follow. Empty means every reference was followed.
    pub not_followed: Vec<NotFollowed>,
}

/// Measures `glue` and every file reached from it by static relative
/// specifiers. `artifact` is the module, which a glue references by
/// `new URL` and which is neither counted again nor reported as skipped.
///
/// # Errors
///
/// Fails if the glue itself cannot be read; a file it references that cannot
/// be found is reported in `not_followed`.
pub fn measure(glue: &Path, artifact: &Path) -> Result<LoadGroup, Error> {
    let read = |path: &Path| {
        std::fs::read(path).map_err(|source| Error::Read {
            path: path.to_path_buf(),
            source,
        })
    };
    let root = std::fs::canonicalize(glue).map_err(|source| Error::Read {
        path: glue.to_path_buf(),
        source,
    })?;
    let dir = root.parent().expect("a file has a parent").to_path_buf();
    let artifact = std::fs::canonicalize(artifact).ok();

    let mut group = LoadGroup {
        files: Vec::new(),
        raw: 0,
        gzip: 0,
        brotli: 0,
        not_followed: Vec::new(),
    };
    let mut seen: HashSet<PathBuf> = HashSet::from([root.clone()]);
    let mut queue = vec![(root, read(glue)?)];
    let mut next = 0;
    while next < queue.len() {
        let (path, bytes) = (queue[next].0.clone(), queue[next].1.clone());
        next += 1;
        let name = relative(&dir, &path);
        group.raw += bytes.len() as u64;
        let (gzip, brotli) = (gzip_len(&bytes), brotli_len(&bytes));
        group.gzip += gzip;
        group.brotli += brotli;
        group.files.push(LoadFile {
            path: name.clone(),
            raw: bytes.len() as u64,
            gzip,
            brotli,
        });
        if !is_script(&path) {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        for reference in scan(&text) {
            let mut skip = |specifier: Option<String>, reason| {
                group.not_followed.push(NotFollowed {
                    from: name.clone(),
                    specifier,
                    reason,
                });
            };
            let (specifier, kind) = match reference {
                Reference::Dynamic(specifier) => {
                    skip(specifier, Reason::Dynamic);
                    continue;
                }
                Reference::Static(specifier, kind) => (specifier, kind),
            };
            let Some(target) = resolve(&path, &specifier, kind) else {
                skip(Some(specifier), Reason::NotRelative);
                continue;
            };
            let Ok(target) = std::fs::canonicalize(&target) else {
                // Not on disk. Outside the directory is still told apart, lexically.
                let reason = if target.starts_with(&dir) {
                    Reason::Missing
                } else {
                    Reason::OutsideDirectory
                };
                skip(Some(specifier), reason);
                continue;
            };
            if Some(&target) == artifact.as_ref() {
                // The module is measured on its own; naming it is not a gap.
            } else if !target.starts_with(&dir) {
                skip(Some(specifier), Reason::OutsideDirectory);
            } else if seen.insert(target.clone()) {
                match std::fs::read(&target) {
                    Ok(bytes) => queue.push((target, bytes)),
                    Err(_) => skip(Some(specifier), Reason::Missing),
                }
            }
        }
    }
    Ok(group)
}

/// Whether the scanner reads the file for further references.
fn is_script(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("js" | "mjs" | "cjs")
    )
}

/// `specifier` as a path next to `from`, lexically normalised; `None` if it is
/// not a file reference (a bare name, an absolute path, a URL).
fn resolve(from: &Path, specifier: &str, kind: Kind) -> Option<PathBuf> {
    let specifier = specifier.split(['?', '#']).next().unwrap_or_default();
    let relative = match kind {
        Kind::Module => specifier.starts_with("./") || specifier.starts_with("../"),
        Kind::Url => !specifier.starts_with('/') && !specifier.contains(':'),
    };
    if !relative || specifier.is_empty() {
        return None;
    }
    let mut path = from.parent().expect("a file has a parent").to_path_buf();
    for part in Path::new(specifier).components() {
        match part {
            Component::ParentDir => {
                path.pop();
            }
            Component::Normal(name) => path.push(name),
            _ => {}
        }
    }
    Some(path)
}

fn relative(dir: &Path, path: &Path) -> String {
    path.strip_prefix(dir)
        .unwrap_or(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn statics(text: &str) -> Vec<String> {
        scan(text)
            .into_iter()
            .filter_map(|r| match r {
                Reference::Static(s, _) => Some(s),
                Reference::Dynamic(_) => None,
            })
            .collect()
    }

    #[test]
    fn finds_imports_and_re_exports_without_semicolons() {
        let text = "import { a, b } from './a.js'\nimport * as c from \"./c.js\"\nimport d from './d.js'\nimport './side.js'\nexport * from './e.js'\nexport { f as g } from './f.js';\nexport * as h from './h.js';";
        assert_eq!(
            statics(text),
            [
                "./a.js",
                "./c.js",
                "./d.js",
                "./side.js",
                "./e.js",
                "./f.js",
                "./h.js"
            ]
        );
    }

    #[test]
    fn an_export_without_from_has_no_reference() {
        assert_eq!(
            statics("export { a, b };\nexport function f() {}\nexport default 1;"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn finds_new_url_with_import_meta_url() {
        let found = scan("x = new URL('m_bg.wasm', import.meta.url);");
        assert_eq!(found, [Reference::Static("m_bg.wasm".into(), Kind::Url)]);
    }

    #[test]
    fn a_computed_new_url_is_dynamic() {
        assert_eq!(
            scan("new URL(name + '.wasm', import.meta.url)"),
            [Reference::Dynamic(None)]
        );
        assert_eq!(scan("new URL('https://x.test/a')"), Vec::<Reference>::new());
    }

    #[test]
    fn dynamic_import_is_not_a_static_reference() {
        assert_eq!(
            scan("const m = await import('./lazy.js'); import(name);"),
            [
                Reference::Dynamic(Some("./lazy.js".into())),
                Reference::Dynamic(None)
            ]
        );
    }

    #[test]
    fn import_meta_is_not_an_import() {
        assert_eq!(scan("const u = import.meta.url;"), Vec::<Reference>::new());
    }

    #[test]
    fn comments_strings_and_templates_hide_references() {
        let text = "// import './a.js'\n/* import './b.js' */\nconst s = \"import './c.js'\";\nconst t = `import './d.js' ${ 'x' + `${1}` } export * from './e.js'`;\nimport './real.js'";
        assert_eq!(statics(text), ["./real.js"]);
    }

    #[test]
    fn a_template_substitution_is_scanned_like_code() {
        assert_eq!(
            statics("`a${ import('./x.js') }b`; import './y.js'").len(),
            1
        );
    }

    #[test]
    fn regex_literals_do_not_swallow_the_rest() {
        let text = "const r = /[\"'`]/g;\nconst q = a / b / c;\nimport './after.js'";
        assert_eq!(statics(text), ["./after.js"]);
    }

    #[test]
    fn a_property_called_import_is_ignored() {
        assert_eq!(
            statics("obj.import('./x.js'); obj.export * from './y.js'"),
            Vec::<String>::new()
        );
    }

    fn project(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("peakmark-unit").join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("snippets")).unwrap();
        dir
    }

    #[test]
    fn follows_recursively_survives_cycles_and_reports_what_it_skips() {
        let dir = project("load-group-unit");
        let write = |name: &str, text: &str| std::fs::write(dir.join(name), text).unwrap();
        write("m.wasm", "\0asm");
        write(
            "m.js",
            "import { a } from './snippets/a.js';\nimport './snippets/a.js';\nimport 'node:fs';\nimport './gone.js';\nimport '../outside.js';\nimport('./lazy.js');\nnew URL('m.wasm', import.meta.url);\nnew URL('data.bin', import.meta.url);",
        );
        write("snippets/a.js", "import '../m.js'; export * from './b.js';");
        write("snippets/b.js", "export const b = 1;");
        write("data.bin", "12345");
        write("../outside.js", "export {}");

        let group = measure(&dir.join("m.js"), &dir.join("m.wasm")).unwrap();
        let names: Vec<_> = group.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            names,
            ["m.js", "snippets/a.js", "data.bin", "snippets/b.js"]
        );
        assert_eq!(group.raw, group.files.iter().map(|f| f.raw).sum::<u64>());
        assert_eq!(
            group.brotli,
            group.files.iter().map(|f| f.brotli).sum::<u64>()
        );
        let skipped: Vec<_> = group
            .not_followed
            .iter()
            .map(|n| (n.specifier.as_deref().unwrap(), n.reason))
            .collect();
        assert_eq!(
            skipped,
            [
                ("node:fs", Reason::NotRelative),
                ("./gone.js", Reason::Missing),
                ("../outside.js", Reason::OutsideDirectory),
                ("./lazy.js", Reason::Dynamic),
            ]
        );
    }

    #[test]
    fn a_missing_glue_is_an_error() {
        let dir = project("load-group-missing");
        assert!(measure(&dir.join("m.js"), &dir.join("m.wasm")).is_err());
    }
}
