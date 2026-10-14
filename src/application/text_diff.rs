//! Line diffs between two spec texts, computed with `similar` (Myers). One
//! implementation serves every diff view: the structured hunks feed the UI's
//! unified and side-by-side renderers, and `render_unified` turns the same
//! hunks into a patch.

use serde::Serialize;
use similar::{DiffOp, TextDiff};

/// What a diff line is relative to the two sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DiffLineOp {
    /// On both sides (equal, or equal apart from whitespace when ignored).
    Ctx,
    /// Only on the new side.
    Add,
    /// Only on the old side.
    Del,
}

/// One line of a hunk. Line numbers are 1-based; a side the line is not on
/// has none. `text` is the new side's text for context lines; `old_text`
/// carries the old side's text when it differs (only possible when
/// whitespace is ignored).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiffLine {
    pub op: DiffLineOp,
    pub old_no: Option<usize>,
    pub new_no: Option<usize>,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_text: Option<String>,
}

/// A contiguous region of changes with its surrounding context, numbered like
/// a unified-diff `@@ -old_start,old_len +new_start,new_len @@` header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiffHunk {
    pub old_start: usize,
    pub old_len: usize,
    pub new_start: usize,
    pub new_len: usize,
    pub lines: Vec<DiffLine>,
}

/// How to compare two texts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiffOptions {
    /// Compare lines without leading and trailing whitespace, so indentation
    /// and trailing-whitespace differences are not changes.
    pub ignore_whitespace: bool,
    /// Unchanged lines kept around each change; `None` keeps the whole text
    /// (one hunk spanning both documents whenever anything changed).
    pub context: Option<usize>,
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            ignore_whitespace: false,
            context: Some(3),
        }
    }
}

/// The line diff between `old` and `new` as hunks; empty when the texts are
/// equal (under the chosen whitespace rule).
pub fn diff_lines<'a>(old: &'a str, new: &'a str, options: DiffOptions) -> Vec<DiffHunk> {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let key = |l: &&'a str| -> &'a str {
        if options.ignore_whitespace {
            l.trim()
        } else {
            l
        }
    };
    let old_keys: Vec<&str> = old_lines.iter().map(key).collect();
    let new_keys: Vec<&str> = new_lines.iter().map(key).collect();
    let diff = TextDiff::from_slices(&old_keys, &new_keys);
    // "Whole text" is a context larger than either side; `usize::MAX` would
    // overflow inside the grouping.
    let radius = options
        .context
        .unwrap_or(old_lines.len() + new_lines.len() + 1);

    diff.grouped_ops(radius)
        .iter()
        .map(|group| hunk(group, &old_lines, &new_lines))
        .collect()
}

fn hunk(group: &[DiffOp], old_lines: &[&str], new_lines: &[&str]) -> DiffHunk {
    let mut lines = Vec::new();
    for op in group {
        match *op {
            DiffOp::Equal {
                old_index,
                new_index,
                len,
            } => {
                for i in 0..len {
                    let old_text = old_lines.get(old_index + i).copied().unwrap_or_default();
                    let new_text = new_lines.get(new_index + i).copied().unwrap_or_default();
                    lines.push(DiffLine {
                        op: DiffLineOp::Ctx,
                        old_no: Some(old_index + i + 1),
                        new_no: Some(new_index + i + 1),
                        text: new_text.to_string(),
                        old_text: (old_text != new_text).then(|| old_text.to_string()),
                    });
                }
            }
            DiffOp::Delete {
                old_index, old_len, ..
            } => push_deleted(&mut lines, old_lines, old_index, old_len),
            DiffOp::Insert {
                new_index, new_len, ..
            } => push_inserted(&mut lines, new_lines, new_index, new_len),
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                push_deleted(&mut lines, old_lines, old_index, old_len);
                push_inserted(&mut lines, new_lines, new_index, new_len);
            }
        }
    }
    let (old_start, old_len) = side_range(group.iter().map(|op| op.old_range()));
    let (new_start, new_len) = side_range(group.iter().map(|op| op.new_range()));
    DiffHunk {
        old_start,
        old_len,
        new_start,
        new_len,
        lines,
    }
}

fn push_deleted(lines: &mut Vec<DiffLine>, old_lines: &[&str], index: usize, len: usize) {
    for i in index..index + len {
        lines.push(DiffLine {
            op: DiffLineOp::Del,
            old_no: Some(i + 1),
            new_no: None,
            text: old_lines.get(i).copied().unwrap_or_default().to_string(),
            old_text: None,
        });
    }
}

fn push_inserted(lines: &mut Vec<DiffLine>, new_lines: &[&str], index: usize, len: usize) {
    for i in index..index + len {
        lines.push(DiffLine {
            op: DiffLineOp::Add,
            old_no: None,
            new_no: Some(i + 1),
            text: new_lines.get(i).copied().unwrap_or_default().to_string(),
            old_text: None,
        });
    }
}

/// Unified-diff numbering of one side: 1-based start and length; an empty
/// side starts at the line before it (GNU convention).
fn side_range(ranges: impl Iterator<Item = std::ops::Range<usize>>) -> (usize, usize) {
    let mut start: Option<usize> = None;
    let mut end = 0;
    for r in ranges {
        start = Some(start.map_or(r.start, |s| s.min(r.start)));
        end = end.max(r.end);
    }
    let start = start.unwrap_or(0);
    let len = end.saturating_sub(start);
    if len == 0 {
        (start, 0)
    } else {
        (start + 1, len)
    }
}

/// The hunks as a unified-diff patch with `---`/`+++` labels; empty when
/// there are no hunks. Context lines are written with the new side's text.
pub fn render_unified(hunks: &[DiffHunk], from_label: &str, to_label: &str) -> String {
    if hunks.is_empty() {
        return String::new();
    }
    let mut out = format!("--- {from_label}\n+++ {to_label}\n");
    for h in hunks {
        out.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            h.old_start, h.old_len, h.new_start, h.new_len
        ));
        for l in &h.lines {
            let sign = match l.op {
                DiffLineOp::Ctx => ' ',
                DiffLineOp::Add => '+',
                DiffLineOp::Del => '-',
            };
            out.push(sign);
            out.push_str(&l.text);
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ops(hunks: &[DiffHunk]) -> Vec<(DiffLineOp, String)> {
        hunks
            .iter()
            .flat_map(|h| h.lines.iter().map(|l| (l.op, l.text.clone())))
            .collect()
    }

    #[test]
    fn equal_texts_have_no_hunks() {
        let hunks = diff_lines("a\nb\n", "a\nb\n", DiffOptions::default());
        assert!(hunks.is_empty());
        assert_eq!(render_unified(&hunks, "x", "y"), "");
    }

    #[test]
    fn a_changed_line_is_one_delete_and_one_insert_with_context() {
        let old = "a\nb\nc\nd\ne\nf\ng\nh\n";
        let new = "a\nb\nc\nd\nE\nf\ng\nh\n";
        let hunks = diff_lines(old, new, DiffOptions::default());
        assert_eq!(hunks.len(), 1);
        let h = &hunks[0];
        assert_eq!(
            (h.old_start, h.old_len, h.new_start, h.new_len),
            (2, 7, 2, 7)
        );
        let changed: Vec<_> = h
            .lines
            .iter()
            .filter(|l| l.op != DiffLineOp::Ctx)
            .map(|l| (l.op, l.old_no, l.new_no, l.text.as_str()))
            .collect();
        assert_eq!(
            changed,
            vec![
                (DiffLineOp::Del, Some(5), None, "e"),
                (DiffLineOp::Add, None, Some(5), "E"),
            ]
        );
    }

    #[test]
    fn whole_text_context_keeps_every_line_in_one_hunk() {
        let old: String = (0..3000).map(|i| format!("line {i}\n")).collect();
        let new = old.replace("line 1500\n", "line 1500 changed\n");
        let options = DiffOptions {
            context: None,
            ..DiffOptions::default()
        };
        let hunks = diff_lines(&old, &new, options);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].lines.len(), 3001);
        let changes = hunks[0]
            .lines
            .iter()
            .filter(|l| l.op != DiffLineOp::Ctx)
            .count();
        assert_eq!(changes, 2, "one changed line: one delete, one insert");
    }

    #[test]
    fn separate_changes_far_apart_form_separate_hunks() {
        let old: String = (0..40).map(|i| format!("{i}\n")).collect();
        let new: String = (0..40)
            .map(|i| match i {
                5 => "five\n".to_string(),
                35 => "thirty-five\n".to_string(),
                _ => format!("{i}\n"),
            })
            .collect();
        let hunks = diff_lines(&old, &new, DiffOptions::default());
        assert_eq!(hunks.len(), 2);
        assert_eq!(hunks[1].old_start, 33);
    }

    #[test]
    fn ignoring_whitespace_hides_indentation_and_trailing_space_changes() {
        let old = "tags:\n- a\nname: x\n";
        let new = "tags:\n  - a\nname: x   \n";
        let strict = diff_lines(old, new, DiffOptions::default());
        assert_eq!(
            ops(&strict)
                .iter()
                .filter(|(op, _)| *op != DiffLineOp::Ctx)
                .count(),
            4
        );

        let lenient = diff_lines(
            old,
            new,
            DiffOptions {
                ignore_whitespace: true,
                ..DiffOptions::default()
            },
        );
        assert!(lenient.is_empty(), "{lenient:?}");
    }

    #[test]
    fn ignoring_whitespace_keeps_real_changes_with_original_indentation() {
        let old = "root:\n- a\n  value: 1\n";
        let new = "root:\n  - a\n    value: 2\n";
        let hunks = diff_lines(
            old,
            new,
            DiffOptions {
                ignore_whitespace: true,
                context: None,
            },
        );
        assert_eq!(
            ops(&hunks),
            vec![
                (DiffLineOp::Ctx, "root:".to_string()),
                (DiffLineOp::Ctx, "  - a".to_string()),
                (DiffLineOp::Del, "  value: 1".to_string()),
                (DiffLineOp::Add, "    value: 2".to_string()),
            ]
        );
        // The context line whose indentation changed keeps both texts.
        assert_eq!(hunks[0].lines[1].old_text.as_deref(), Some("- a"));
        assert_eq!(hunks[0].lines[0].old_text, None);
    }

    #[test]
    fn additions_to_an_empty_text_number_the_old_side_from_zero() {
        let hunks = diff_lines("", "a\nb\n", DiffOptions::default());
        assert_eq!(hunks.len(), 1);
        let h = &hunks[0];
        assert_eq!(
            (h.old_start, h.old_len, h.new_start, h.new_len),
            (0, 0, 1, 2)
        );
    }

    #[test]
    fn render_unified_writes_labels_hunk_headers_and_signed_lines() {
        let hunks = diff_lines("a\nb\n", "a\nc\n", DiffOptions::default());
        assert_eq!(
            render_unified(&hunks, "svc 1.0.0", "svc 1.1.0"),
            "--- svc 1.0.0\n+++ svc 1.1.0\n@@ -1,2 +1,2 @@\n a\n-b\n+c\n"
        );
    }

    #[test]
    fn hunks_serialize_with_lowercase_ops_and_optional_old_text() {
        let hunks = diff_lines("a\n", "b\n", DiffOptions::default());
        let json = serde_json::to_value(&hunks).unwrap_or_default();
        assert_eq!(json[0]["lines"][0]["op"], "del");
        assert_eq!(json[0]["lines"][1]["op"], "add");
        assert!(json[0]["lines"][0].get("old_text").is_none());
    }
}
