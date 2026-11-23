// Tests for what the authored pages under site/ say: examples and layouts a reader relies on.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const SITE = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../site");

/** The text of an authored page under site/. */
const page = (/** @type {string} */ rel) => fs.readFileSync(path.join(SITE, rel), "utf8");

void test("the complete sanshain.yaml example shows autoGa switched off, with what true does", () => {
  const html = page("developer/sanshain-yaml.html");
  const start = html.indexOf("<h2>A complete example</h2>");
  assert.notEqual(start, -1, "the page has a complete example");
  const example = /<pre><code>([\s\S]*?)<\/code><\/pre>/.exec(html.slice(start));
  assert.ok(example, "the complete example is a code block");
  const line = (example[1] ?? "").split("\n").find((l) => l.startsWith("autoGa:"));
  assert.equal(
    line,
    "autoGa: false                        # true: GA on trunk/tag builds (needs releaser)",
  );
});
