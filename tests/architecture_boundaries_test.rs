//! The DDD layering in AGENTS.md is a rule humans and agents have been asked
//! to keep by hand. This makes it fail the build instead: domain and
//! application must not reach for a framework or an adapter, because the
//! moment they do, the use-case logic can only be tested through a database
//! and a web server.
//!
//! Deliberately textual rather than a dependency-graph tool: it needs no extra
//! toolchain, runs in the normal `cargo test` everyone already runs, and the
//! failure names the file and the token so the fix is obvious.

use std::fs;
use std::path::{Path, PathBuf};

/// Tokens that mean "a framework or adapter leaked into this layer". Matched
/// as whole words against code, never comments — a doc comment explaining the
/// rule ("no Axum here") must not trip the rule it explains.
struct Boundary {
    dir: &'static str,
    forbidden: &'static [&'static str],
    why: &'static str,
}

const BOUNDARIES: &[Boundary] = &[
    Boundary {
        dir: "src/domain",
        forbidden: &[
            "axum",
            "sqlx",
            "ldap3",
            "moka",
            "tracing_subscriber",
            "crate::infrastructure",
            "crate::presentation",
            "crate::application",
        ],
        why: "the domain holds models and port traits only — it must not know how anything is served or stored",
    },
    Boundary {
        dir: "src/application",
        forbidden: &[
            "axum",
            "sqlx",
            "ldap3",
            "crate::infrastructure",
            "crate::presentation",
        ],
        why: "use cases depend on the domain's ports, not on a concrete adapter or the web layer",
    },
];

#[cfg(test)]
fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => panic!("Failed to read {}: {e}", dir.display()),
    };
    for entry in entries {
        let path = entry
            .unwrap_or_else(|e| panic!("bad entry in {}: {e}", dir.display()))
            .path();
        if path.is_dir() {
            out.extend(rust_files(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    out
}

/// Strip line comments so prose about a forbidden token is not itself a
/// violation. Block comments are left alone: none of the layered code uses
/// them, and a half-parser would be worse than none.
#[cfg(test)]
fn code_only(line: &str) -> &str {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") {
        return "";
    }
    match line.find("//") {
        Some(idx) => &line[..idx],
        None => line,
    }
}

#[cfg(test)]
fn mentions_word(haystack: &str, word: &str) -> bool {
    let mut from = 0;
    while let Some(idx) = haystack[from..].find(word) {
        let start = from + idx;
        let end = start + word.len();
        let before_ok = start == 0
            || !haystack[..start]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_');
        let after_ok = !haystack[end..]
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || c == '_');
        if before_ok && after_ok {
            return true;
        }
        from = end;
    }
    false
}

#[test]
fn layers_do_not_reach_across_their_boundary() {
    let mut violations: Vec<String> = Vec::new();

    for boundary in BOUNDARIES {
        for file in rust_files(Path::new(boundary.dir)) {
            let contents = fs::read_to_string(&file)
                .unwrap_or_else(|e| panic!("Failed to read {}: {e}", file.display()));
            for (number, line) in contents.lines().enumerate() {
                let code = code_only(line);
                for token in boundary.forbidden {
                    if mentions_word(code, token) {
                        violations.push(format!(
                            "{}:{}: `{}` — {}",
                            file.display(),
                            number + 1,
                            token,
                            boundary.why
                        ));
                    }
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "architecture boundary violated:\n  {}\n\nMove the dependency behind a port in \
         src/domain/ports.rs and implement it in src/infrastructure/, or keep the logic in \
         the layer that already owns it.",
        violations.join("\n  ")
    );
}

/// The rule above is only worth having if it can fail, and a textual check is
/// exactly the kind that quietly stops matching. This pins the matcher itself.
#[test]
fn the_boundary_check_detects_a_real_import() {
    assert!(mentions_word("use axum::Json;", "axum"));
    assert!(mentions_word("    pool: sqlx::SqlitePool,", "sqlx"));
    assert!(mentions_word(
        "use crate::infrastructure::x;",
        "crate::infrastructure"
    ));
    // Not a violation: a word that merely contains the token.
    assert!(!mentions_word("let axumlike = 1;", "axum"));
    assert!(!mentions_word("struct Sqlxish;", "sqlx"));
    // Not a violation: prose about the rule.
    assert_eq!(code_only("// no axum in this layer"), "");
    assert_eq!(code_only("/// sqlx must not appear here"), "");
    assert!(!mentions_word(
        code_only("    // uses axum elsewhere"),
        "axum"
    ));
}
