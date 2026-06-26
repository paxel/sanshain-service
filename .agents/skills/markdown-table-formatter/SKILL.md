---
name: markdown-table-formatter
description: Enforce consistent alignment and pretty-printing of Markdown tables.
---

# Markdown Table Formatter

Use this skill to ensure all Markdown tables are properly aligned, making them readable in raw text format.

## Trigger
- "align all tables in [file]"
- "pretty-print markdown tables"
- "fix table formatting"

## Procedures

### 1. Align Table Borders
- All column borders (`|`) must be aligned vertically at the same character column across all rows of the table.
- Use at least one space of padding between the content and the pipe `|` characters.

### 2. Header Separator
- The separator row (between header and body) should match the width of the columns.
- Use at least three hyphens `---` for each column.

### 3. Example

**Before:**
| Header 1 | Header 2 |
|---|---|
| Value | Short |
| Much longer value | Long |

**After:**
| Header 1          | Header 2 |
|-------------------|----------|
| Value             | Short    |
| Much longer value | Long     |

## Guidelines
- Avoid excessive trailing spaces.
- If a column is empty, maintain the alignment with spaces.
- For tables with many columns, consider if they still fit within reasonable line length limits (~120 chars), but prioritize alignment.

## Quality Checklist
- [ ] Are all `|` characters vertically aligned?
- [ ] Does the separator row use `---` (not just `-`) and match the column width?
- [ ] Is there exactly one space of padding on each side of the content?
