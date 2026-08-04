//! Static asset weight.
//!
//! The logo is painted into a 66x36 navigation slot and a 234x128 hero, and the
//! light-mode file is also the favicon, so it ships on every page load. Shipping
//! a multi-megabyte source export for that is invisible in review — nothing
//! fails, the page just gets slower — so the ceiling is asserted here instead.
//!
//! The ceilings are deliberately generous. They guard against recommitting an
//! unoptimised export; they are not targets to tune against.
//!
//! Note on the dark logo: it used to carry an alpha channel, and the intent was
//! to keep it. Measured across all 1,081,344 pixels of the original its alpha
//! was 255 everywhere — fully opaque — so it encoded nothing and cost a quarter
//! of the pixel data. It is now stored without one. Anything that later needs
//! genuine transparency here has to reinstate it deliberately.

use std::path::{Path, PathBuf};

/// The two theme logos, swapped by the light/dark toggle.
const LOGOS: [&str; 2] = ["sonne.png", "socke.png"];

/// Largest real use is the landing hero at 234px wide; 2x that is 468. The
/// ceiling leaves room without allowing a full-resolution source image back in.
const MAX_LOGO_EDGE_PX: u32 = 512;

/// Combined ceiling for the two theme logos. They were 3.2 MB together and are
/// now ~123 KB.
///
/// Both are photographic — ~92k distinct colours — so a 468px PNG lands around
/// 50-75 KB each at a 128-colour palette. Going lower starts to band the sky
/// gradients, which is where the remaining bytes are. Set from that measurement
/// with headroom, not from a round number picked in advance.
const MAX_LOGOS_TOTAL_BYTES: u64 = 160 * 1024;

/// A favicon is drawn at 16-32px, so 64px covers it at 2x. Measured at ~4 KB;
/// the ceiling is loose enough to survive a re-export and tight enough that
/// pointing the icon link back at a full logo would fail.
const MAX_FAVICON_BYTES: u64 = 12 * 1024;
const MAX_FAVICON_EDGE_PX: u32 = 128;

/// A redirect stub — `<title>Redirecting...</title>`, no head furniture — so it
/// carries no icon link and is exempt from the per-page check below.
const PAGES_WITHOUT_AN_ICON_LINK: [&str; 1] = ["service.html"];

fn logo_path(name: &str) -> PathBuf {
    Path::new("static/images").join(name)
}

/// Width and height from a PNG's IHDR chunk.
///
/// Read directly rather than pulling in an image crate: the layout is fixed —
/// 8-byte signature, 4-byte chunk length, the `IHDR` tag, then width and height
/// as big-endian u32. Note the colour-type byte lives at index 25, outside the
/// length check below; widen it before reading anything past the dimensions.
// `cfg(test)` is always true here; the attribute marks the helper as test code
// for clippy's `allow-panic-in-tests`, matching the idiom in middleware_test.rs.
#[cfg(test)]
fn png_dimensions(path: &Path) -> (u32, u32) {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    assert!(
        bytes.len() > 24,
        "{} is too short to be a PNG",
        path.display()
    );
    assert_eq!(&bytes[1..4], b"PNG", "{} is not a PNG", path.display());
    assert_eq!(&bytes[12..16], b"IHDR", "{} has no IHDR", path.display());

    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    (width, height)
}

// `cfg(test)` is always true here; the attribute marks the helper as test code
// for clippy's `allow-panic-in-tests`, matching the idiom in middleware_test.rs.
#[cfg(test)]
fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path)
        .unwrap_or_else(|e| panic!("stat {}: {e}", path.display()))
        .len()
}

/// Every page that renders the banner, plus the Askama layout, which lives
/// outside `static/`.
#[cfg(test)]
fn pages_with_a_head() -> Vec<PathBuf> {
    let mut pages: Vec<PathBuf> = std::fs::read_dir("static")
        .expect("read static/")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("html"))
        .collect();
    pages.push(PathBuf::from("templates/layout.html"));
    pages.sort();
    pages
}

#[test]
fn theme_logos_are_sized_for_how_they_are_displayed() {
    for name in LOGOS {
        let (w, h) = png_dimensions(&logo_path(name));
        assert!(
            w <= MAX_LOGO_EDGE_PX && h <= MAX_LOGO_EDGE_PX,
            "{name} is {w}x{h}; the largest place it is drawn is 234x128, so anything \
             over {MAX_LOGO_EDGE_PX}px on an edge is being downscaled by the browser"
        );
    }
}

#[test]
fn theme_logos_are_small_enough_to_ship_on_every_page() {
    let total: u64 = LOGOS.iter().map(|n| file_size(&logo_path(n))).sum();

    assert!(
        total <= MAX_LOGOS_TOTAL_BYTES,
        "the two theme logos total {total} bytes, over the {MAX_LOGOS_TOTAL_BYTES} ceiling. \
         The light logo loads on every page and the dark one on the first theme toggle, \
         so this is paid by every visitor."
    );
}

#[test]
fn a_dedicated_favicon_exists() {
    assert!(
        logo_path("favicon.png").exists(),
        "a dedicated favicon must exist; pointing the icon link at the full logo \
         makes every page load fetch it"
    );
}

#[test]
fn the_favicon_is_sized_for_a_browser_tab() {
    let path = logo_path("favicon.png");

    let size = file_size(&path);
    assert!(
        size <= MAX_FAVICON_BYTES,
        "favicon.png is {size} bytes, over the {MAX_FAVICON_BYTES} ceiling"
    );

    let (w, h) = png_dimensions(&path);
    assert!(
        w <= MAX_FAVICON_EDGE_PX && h <= MAX_FAVICON_EDGE_PX,
        "favicon.png is {w}x{h}; a favicon is drawn at 16-32px"
    );
}

#[test]
fn every_page_points_its_icon_at_the_favicon() {
    let mut problems = Vec::new();

    for path in pages_with_a_head() {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        let html = std::fs::read_to_string(&path).expect("read html");

        let links: Vec<&str> = html
            .lines()
            .filter(|l| l.contains("rel=\"icon\""))
            .collect();

        // Counting per page rather than in total: a total would let one page
        // lose its icon link entirely while the sum still looked healthy.
        if PAGES_WITHOUT_AN_ICON_LINK.contains(&name.as_str()) {
            if !links.is_empty() {
                problems.push(format!(
                    "{name} is listed as having no icon link, but has {}",
                    links.len()
                ));
            }
            continue;
        }

        match links.len() {
            1 => {
                if !links[0].contains("favicon.png") {
                    problems.push(format!(
                        "{name} points its icon elsewhere: {}",
                        links[0].trim()
                    ));
                }
            }
            0 => problems.push(format!(
                "{name} has no icon link; add one, or list it in PAGES_WITHOUT_AN_ICON_LINK"
            )),
            n => problems.push(format!("{name} has {n} icon links, expected exactly one")),
        }
    }

    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
