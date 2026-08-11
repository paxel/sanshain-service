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
