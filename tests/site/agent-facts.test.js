// The agents' fact map (docs/agents/service-facts.md) points at the files that
// are the authority for each fact. A pointer to a file that was moved or
// deleted sends an agent to nothing, so every path it names must exist.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const MAP = "docs/agents/service-facts.md";

// Backticked repository paths: under a top-level directory of the repository,
// or one of the root files the map names. Globs (`tests/*.rs`) are patterns,
// not pointers, and are left out.
function pointers(text) {
  const found = new Set();
  const dirs = "src|docs|site|static|tests|scripts|templates|deploy|\\.github";
  const files =
    "CONTEXT\\.md|AGENTS\\.md|CHANGELOG\\.md|OLDER_CHANGES\\.md|api\\.yaml|maintenance\\.yaml";
  const pattern = new RegExp(`\`((?:(?:${dirs})/[^\`\\s]*)|(?:${files}))\``, "g");
  for (const match of text.matchAll(pattern)) {
    if (!match[1].includes("*")) found.add(match[1]);
  }
  return [...found];
}

test("every path the agents' fact map names exists", () => {
  const text = fs.readFileSync(path.join(root, MAP), "utf8");
  const named = pointers(text);
  assert.ok(named.length > 20, `expected the map to name its sources, found ${named.length}`);
  const missing = named.filter((p) => !fs.existsSync(path.join(root, p)));
  assert.deepEqual(missing, [], `${MAP} points at paths that do not exist`);
});

test("the pointer reader finds paths and skips globs", () => {
  const sample = "see `src/lib.rs`, `CONTEXT.md`, `tests/*.rs` and `san_…`";
  assert.deepEqual(pointers(sample).sort(), ["CONTEXT.md", "src/lib.rs"]);
});
