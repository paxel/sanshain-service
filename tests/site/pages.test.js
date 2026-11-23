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

const CLIENT_PAGES = ["maven", "rust", "go", "javascript", "conan"];

/** The Options section of a client page: from its heading to the next h2 or the end. */
function optionsSection(/** @type {string} */ html) {
  const start = html.indexOf('<h2 id="options">');
  assert.notEqual(start, -1, "the page has an Options section");
  const next = html.indexOf("<h2", start + 1);
  return html.slice(start, next === -1 ? undefined : next);
}

for (const client of CLIENT_PAGES) {
  void test(`the ${client} page lists its options as one block each, not as a table`, () => {
    const section = optionsSection(page(`developer/clients/${client}.html`));
    assert.doesNotMatch(section, /<table/);
    const blocks = section.split('<div class="option">').slice(1);
    assert.ok(blocks.length > 20, `${client} keeps its options (${String(blocks.length)})`);
    for (const block of blocks) {
      assert.match(block, /<p class="option-name">[\s\S]+?<\/p>/);
      const facts = [...block.matchAll(/<dt>([\s\S]*?)<\/dt>/g)].map((m) =>
        (m[1] ?? "").replace(/<\/?code>/g, ""),
      );
      assert.deepEqual(facts, ["Environment", "sanshain.yaml", "Default", "Since"]);
      assert.match(block, /<p class="option-meaning">[\s\S]+?<\/p>/);
    }
  });
}
