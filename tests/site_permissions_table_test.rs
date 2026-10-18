//! The permission table of the site cannot drift from the `Permission` enum.
//!
//! `site/operator/roles.html` lists every permission by its wire name, one row
//! per permission. This test compares the first-cell names of that table with
//! `Permission::ALL` serialised the way the API serialises it (serde,
//! `snake_case`) and fails, naming the permissions, when either side has one
//! the other lacks. Prior art: the contract checks in `src/lib.rs`.

use regex::Regex;
use sanshain_service::domain::permissions::Permission;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

const TABLE_PAGE: &str = "site/operator/roles.html";
const TABLE_ID: &str = "permissions";

fn read_page() -> Result<String, String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(TABLE_PAGE);
    fs::read_to_string(path).map_err(|e| format!("cannot read {TABLE_PAGE}: {e}"))
}

/// The `<code>` text of the first cell of every body row of the table with `id`.
fn first_cell_codes(html: &str, id: &str) -> Result<BTreeSet<String>, String> {
    let open = format!("<table id=\"{id}\">");
    let start = html
        .find(&open)
        .ok_or_else(|| format!("{TABLE_PAGE}: no table with id=\"{id}\""))?;
    let rest = &html[start + open.len()..];
    let end = rest
        .find("</table>")
        .ok_or_else(|| format!("{TABLE_PAGE}: table id=\"{id}\" is not closed"))?;
    let table = &rest[..end];
    let row = Regex::new(r"(?s)<tr[^>]*>(.*?)</tr>").map_err(|e| e.to_string())?;
    // Anchored at the start of the row's cells: only the first <td> counts.
    let first_code =
        Regex::new(r"(?s)^\s*<td[^>]*>\s*<code>([^<]+)</code>").map_err(|e| e.to_string())?;
    let tags = Regex::new(r"<[^>]*>").map_err(|e| e.to_string())?;
    let mut names = BTreeSet::new();
    for caps in row.captures_iter(table) {
        let cells = &caps[1];
        if cells.contains("<th") {
            continue; // the header row
        }
        let code = first_code
            .captures(cells)
            .map(|c| c[1].trim().to_string())
            .ok_or_else(|| {
                let text = tags.replace_all(cells, " ");
                format!(
                    "{TABLE_PAGE}: a row of table id=\"{id}\" has no <code> in its first cell: {}",
                    text.split_whitespace().collect::<Vec<_>>().join(" ")
                )
            })?;
        names.insert(code);
    }
    Ok(names)
}

/// `Permission::ALL` as the API writes it: the JSON string without its quotes.
fn wire_names() -> Result<BTreeSet<String>, String> {
    Permission::ALL
        .iter()
        .map(|permission| {
            let json = serde_json::to_string(permission).map_err(|e| e.to_string())?;
            Ok(json.trim_matches('"').to_string())
        })
        .collect()
}

#[test]
fn permission_table_lists_exactly_the_permissions_of_the_enum() -> Result<(), String> {
    let documented = first_cell_codes(&read_page()?, TABLE_ID)?;
    let in_code = wire_names()?;

    let undocumented: Vec<&String> = in_code.difference(&documented).collect();
    let unknown: Vec<&String> = documented.difference(&in_code).collect();

    let mut problems = Vec::new();
    if !undocumented.is_empty() {
        problems.push(format!(
            "permissions in Permission::ALL that table id=\"{TABLE_ID}\" in {TABLE_PAGE} does not list: {}",
            undocumented.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
        ));
    }
    if !unknown.is_empty() {
        problems.push(format!(
            "rows of table id=\"{TABLE_ID}\" in {TABLE_PAGE} that are not a Permission: {}",
            unknown
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{}.\nEdit the table in {TABLE_PAGE} (and src/domain/permissions.rs if the enum is wrong).",
            problems.join("; ")
        ))
    }
}

#[test]
fn wire_names_are_snake_case() -> Result<(), String> {
    let snake = Regex::new(r"^[a-z0-9_]+$").map_err(|e| e.to_string())?;
    let names = wire_names()?;
    let odd: Vec<&String> = names.iter().filter(|n| !snake.is_match(n)).collect();
    if names.is_empty() {
        Err("Permission::ALL serialised to no names".to_string())
    } else if odd.is_empty() {
        Ok(())
    } else {
        Err(format!("wire names that are not snake_case: {odd:?}"))
    }
}
