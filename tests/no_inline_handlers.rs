//! Completeness guard for the CSP de-inlining (ai/improvements.md #11).
//!
//! `script-src 'self'` forbids inline scripts *and* inline event-handler
//! attributes, so no served markup may carry an `on*=` handler — every control
//! routes through the delegated dispatcher in `common.js` via `data-<event>`
//! attributes. A UI test only sees a missed handler if it happens to click
//! that control; this test instead scans the source of every static HTML page
//! and page script, so a reintroduced inline handler fails the build
//! regardless of interaction or seeded data.

use std::fs;
use std::path::Path;

/// Inline event-handler attribute names. A handler is the name immediately
/// followed by `=` and a quote — `onclick="..."` or `onclick='...'`.
const EVENTS: &[&str] = &[
    "onclick",
    "onchange",
    "oninput",
    "onsubmit",
    "onkeydown",
    "onkeyup",
    "onkeypress",
    "onmouseover",
    "onmouseout",
    "onmousedown",
    "onmouseup",
    "ondblclick",
    "onfocus",
    "onblur",
    "onload",
    "onerror",
    "onwheel",
    "onscroll",
];

fn scan(dir: &Path, ext: &str, hits: &mut Vec<String>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Third-party bundles are exempt: they are not served with inline
            // handlers that we author, and minified code trips the substring
            // match. The CSP still governs them at runtime.
            if path.file_name().and_then(|n| n.to_str()) == Some("vendor") {
                continue;
            }
            scan(&path, ext, hits);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some(ext) {
            continue;
        }
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for (idx, line) in content.lines().enumerate() {
            for ev in EVENTS {
                if line.contains(&format!("{ev}=\"")) || line.contains(&format!("{ev}='")) {
                    hits.push(format!("{}:{} — {}", path.display(), idx + 1, line.trim()));
                }
            }
        }
    }
}

#[test]
fn no_inline_event_handlers_in_static_assets() {
    let mut hits = Vec::new();
    // Both the static pages and the Askama templates (index/dashboard/layout,
    // served at `/` and `/dashboard`) are governed by the same CSP.
    scan(Path::new("static"), "html", &mut hits);
    scan(Path::new("templates"), "html", &mut hits);
    scan(Path::new("static/js"), "js", &mut hits);
    assert!(
        hits.is_empty(),
        "inline event handlers found — the CSP forbids them, wire the control \
         through a data-<event> attribute instead:\n{}",
        hits.join("\n")
    );
}

/// `script-src 'self'` blocks inline `<script>` blocks as well as handlers, so
/// no page may carry a bare `<script>…</script>`; every script is an external
/// `<script src=…>`. (Inline `<style>` is untouched — `style-src` still allows
/// it, see [`crate`-level note in the CSP].)
fn scan_inline_scripts(dir: &Path, hits: &mut Vec<String>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_inline_scripts(&path, hits);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("html") {
            continue;
        }
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for (idx, line) in content.lines().enumerate() {
            // A bare `<script>` open tag is an inline block; `<script src=…>`
            // and `<script type=…>` carry attributes and are exempt.
            if line.contains("<script>") {
                hits.push(format!("{}:{} — {}", path.display(), idx + 1, line.trim()));
            }
        }
    }
}

/// Collect the function names a page could dispatch to: top-level `function`
/// declarations (which become properties of `window` in a classic script) and
/// any explicit `window.<name> =` export. A `const foo = () => …` at top level
/// is a global *lexical* binding, not a `window` property, so it is
/// deliberately excluded — the dispatcher resolves targets via `window[name]`
/// and would not find it.
fn collect_global_functions(dir: &Path, names: &mut std::collections::HashSet<String>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().and_then(|n| n.to_str()) == Some("vendor") {
                continue;
            }
            collect_global_functions(&path, names);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("js") {
            continue;
        }
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for line in content.lines() {
            // Top-level function declaration: no leading indentation.
            if line == line.trim_start() {
                let decl = line
                    .strip_prefix("async function ")
                    .or_else(|| line.strip_prefix("function "));
                if let Some(rest) = decl {
                    let name: String = rest
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
                        .collect();
                    if !name.is_empty() {
                        names.insert(name);
                    }
                }
            }
            // Explicit window export, at any indentation: `window.foo =`.
            if let Some(idx) = line.find("window.") {
                let rest = &line[idx + "window.".len()..];
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
                    .collect();
                let after = rest[name.len()..].trim_start();
                if !name.is_empty() && after.starts_with('=') && !after.starts_with("==") {
                    names.insert(name);
                }
            }
        }
    }
}

/// Collect every `data-<event>="name"` dispatch target from a page tree.
fn collect_dispatch_targets(dir: &Path, targets: &mut Vec<(String, String)>) {
    const EVENT_ATTRS: &[&str] = &[
        "data-click=\"",
        "data-change=\"",
        "data-input=\"",
        "data-submit=\"",
        "data-keydown=\"",
    ];
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().and_then(|n| n.to_str()) == Some("vendor") {
                continue;
            }
            collect_dispatch_targets(&path, targets);
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str());
        if ext != Some("html") && ext != Some("js") {
            continue;
        }
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for attr in EVENT_ATTRS {
            let mut rest = content.as_str();
            while let Some(pos) = rest.find(attr) {
                rest = &rest[pos + attr.len()..];
                if let Some(end) = rest.find('"') {
                    let value = &rest[..end];
                    // A value carrying `${` is a template expression, not a
                    // literal function name — skip (its resolution is dynamic).
                    if !value.is_empty() && !value.contains("${") {
                        targets.push((path.display().to_string(), value.to_string()));
                    }
                    rest = &rest[end..];
                }
            }
        }
    }
}

/// Every `data-<event>` dispatch target must name a function the delegated
/// dispatcher can resolve on `window` — a top-level `function` or an explicit
/// `window.x =` export. This is the data-independent guard for the failure the
/// runtime resolves-test can only catch on pages it happens to load: a target
/// that is a typo, or defined as a top-level `const`/`let` (not on `window`),
/// is a silent dead button.
#[test]
fn every_dispatch_target_resolves_to_a_global_function() {
    let mut names = std::collections::HashSet::new();
    collect_global_functions(Path::new("static/js"), &mut names);

    let mut targets = Vec::new();
    collect_dispatch_targets(Path::new("static"), &mut targets);
    collect_dispatch_targets(Path::new("templates"), &mut targets);

    let unresolved: Vec<String> = targets
        .into_iter()
        .filter(|(_, name)| !names.contains(name))
        .map(|(file, name)| format!("{file}: data-*=\"{name}\""))
        .collect();

    assert!(
        unresolved.is_empty(),
        "dispatch targets that do not resolve to a global function (typo, or \
         declared as a top-level const/let that is not on window):\n{}",
        unresolved.join("\n")
    );
}

#[test]
fn no_inline_script_blocks_in_pages() {
    let mut hits = Vec::new();
    scan_inline_scripts(Path::new("static"), &mut hits);
    scan_inline_scripts(Path::new("templates"), &mut hits);
    assert!(
        hits.is_empty(),
        "inline <script> blocks found — the CSP forbids them, move the script to \
         an external file under static/js and reference it with <script src=…>:\n{}",
        hits.join("\n")
    );
}
