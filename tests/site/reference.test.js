// Tests for the YAML subset reader and the REST reference generated from the
// contracts: fixtures, the two real contract files against the router, and
// the rendered pages in a real site build.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parseYaml } from "../../scripts/site/yaml.js";
import {
  authLabel,
  loadContract,
  markdownToHtml,
  methodAnchor,
  operations,
  pathGroup,
  pathSlug,
  permissionOf,
  propertyRows,
  renderContract,
  resolveRef,
  schemaExample,
  typeLabel,
} from "../../scripts/site/reference.js";
import { buildSite, DEFAULT_CONTRACTS, readVersion } from "../../scripts/build-site.js";

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const LIB_RS = fs.readFileSync(path.join(REPO_ROOT, "src/lib.rs"), "utf8");

/** Paths and methods registered in create_app(), read the way src/lib.rs's contract tests read them. */
function routerRoutes() {
  const routes = new Map();
  for (const line of LIB_RS.split("\n")) {
    const m = /^\s*\.route\("([^"]+)",(.*)$/.exec(line);
    if (!m) continue;
    const methods = routes.get(m[1]) || new Set();
    for (const v of m[2].matchAll(/\b(get|post|put|patch|delete)\(/g)) methods.add(v[1]);
    routes.set(m[1], methods);
  }
  return routes;
}

const snake = (camel) => camel.replace(/([a-z])([A-Z])/g, "$1_$2").toLowerCase();

/** The guard of every `METHOD /admin/...` route: permission, scope kind, and whether API tokens are admitted. */
function routerGuards() {
  const guards = new Map();
  for (const line of LIB_RS.split("\n")) {
    const m = /^\s*\.route\("(\/admin\/[^"]+)",(.*)$/.exec(line);
    if (!m) continue;
    const g =
      /RouteGuard::(Authenticated|Global\(Permission::(\w+)\)|Producer\(Permission::(\w+)\))/.exec(
        m[2],
      );
    assert.ok(g, `guard on ${m[1]}`);
    const guard = {
      permission: g[2] || g[3] ? snake(g[2] || g[3]) : "any signed-in caller",
      scoped: Boolean(g[3]),
      apiTokens: m[2].includes("require_with_api_tokens"),
    };
    for (const v of m[2].matchAll(/\b(get|post|put|patch|delete)\(/g)) {
      guards.set(`${v[1]} ${m[1]}`, guard);
    }
  }
  return guards;
}

/** The endpoint blocks of a built reference page: method, path, auth chips, permission, scope. */
function endpointBlocks(html) {
  return html
    .split('<section class="endpoint">')
    .slice(1)
    .map((section) => {
      const head =
        /<h3 id="[^"]+"><span class="method method-(\w+)">\w+<\/span> <code>([^<]+)<\/code>/.exec(
          section,
        );
      const auth = /<p class="endpoint-auth">([\s\S]*?)<\/p>/.exec(section)[1];
      const perm =
        /Permission: <code(?: class="gap")?>([^<]+)<\/code> · Scope: (?:<code class="gap">)?([^<]+?)(?:<\/code>)?$/.exec(
          auth,
        );
      return {
        method: head[1],
        path: head[2],
        chips: [...auth.matchAll(/<span class="chip">([^<]+)<\/span>/g)].map((c) => c[1]),
        permission: perm ? perm[1] : null,
        scope: perm ? perm[2] : null,
      };
    });
}

const FIXTURE_CONTRACT = `openapi: "3.0.3"
info:
  title: Fixture API
  version: 1.0
  description: |
    First paragraph with \`code\` and **bold**.

    See [the guide](docs/guide.md) and [the web](https://example.org/).
paths:
  /things/{id}:
    parameters:
      - name: id
        in: path
        required: true
        schema: { type: string }
        description: The thing.
    get:
      summary: Read a thing
      x-auth: session
      description: |
        Session-authenticated. Lists:
        - one
          continued
        - two
      parameters:
        - name: verbose
          in: query
          required: false
          schema: { type: boolean, default: false }
          description: More fields.
      responses:
        "200":
          description: The thing.
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/Thing"
        "401":
          description: No valid session.
        "404":
          description: Gone.
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/Thing"
              example:
                name: "recorded-thing"
                kind: small
    delete:
      summary: 42
      x-permission: manage_things
      x-scope: producer
      description: Requires the \`manage_things\` permission, **or** maintainership of the Producer named in the path.
      responses:
        "204":
          description: Gone.
        "403":
          description: Nope.
components:
  schemas:
    Thing:
      type: object
      required: [name]
      description: A thing.
      properties:
        name:
          type: string
        kind:
          type: string
          enum: [big, small]
          default: big
        when:
          type: string
          format: date-time
          nullable: true
        owner:
          type: object
          properties:
            team:
              type: string
        tags:
          type: array
          items:
            type: string
        stage:
          allOf:
            - $ref: "#/components/schemas/Stage"
          description: Omitted means draft.
        parent:
          $ref: "#/components/schemas/Thing"
    Stage:
      type: string
      enum: [draft, final]
`;

// An administrative fixture: one operation declares no permission.
const ADMIN_FIXTURE = `openapi: "3.0.3"
info:
  title: Admin fixture
  version: "1"
paths:
  /admin/widgets:
    get:
      summary: "Read: widgets"
      x-permission: any-signed-in
      x-scope: global
      description: Any signed-in caller.
      responses:
        "200":
          description: ok
    post:
      summary: "Create: widgets"
      description: Nobody wrote the permission sentence.
      responses:
        "201":
          description: made
        "403":
          description: no
  /admin/widgets/{id}:
    delete:
      summary: "Delete: widgets"
      x-permission: manage_widgets
      x-scope: global
      description: Requires the \`manage_widgets\` permission.
      responses:
        "204":
          description: gone
        "403":
          description: no
  /admin/other:
    get:
      summary: other
      x-permission: any-signed-in
      x-scope: global
      description: Any signed-in caller.
      responses:
        "200":
          description: ok
`;

let tmp;
let fixtureFile;
let realOut;

const contractFile = (name, text) => {
  const file = path.join(tmp, name);
  fs.writeFileSync(file, text);
  return file;
};

const render = (file, extra = {}) =>
  renderContract(loadContract(file), {
    title: "Fixture",
    url: "reference/fixture.html",
    order: 9,
    summary: "s",
    repoUrl: "https://github.com/x/y",
    usageUrl: "api-usage.html",
    ...extra,
  });

before(() => {
  tmp = fs.mkdtempSync(path.join(os.tmpdir(), "sanshain-ref-"));
  fixtureFile = contractFile("fixture.yaml", FIXTURE_CONTRACT);
  realOut = path.join(tmp, "real-out");
  buildSite({ srcDir: path.join(REPO_ROOT, "site"), outDir: realOut, version: "2.6.0" });
});

after(() => {
  fs.rmSync(tmp, { recursive: true, force: true });
});

const built = (rel) => fs.readFileSync(path.join(realOut, rel), "utf8");

// ---------------------------------------------------------------------------
// YAML subset reader

test("yaml: mappings, sequences, scalars, quoting, flow collections and comments", () => {
  const doc = parseYaml(`# leading comment
a: 1
b: "two \\"quoted\\" \\u00e9"
c: 'it''s'
d: Don't use this # c
e: ~
f: null
g: -3.5
h: plain text with: colon and (#47) # a comment
i: value # trailing comment
j: TRUE
k: Null
l: False
m: "1.0"
list:
  - x
  - 2
  - key: v
    other: w
  -
    nested: deep
  - [1, "two", three, "a # b"] # comment after flow
  - { type: boolean, default: false }
empty_list: []
empty_map: {}
"200":
  description: ok
url: http://localhost:3000
`);
  assert.deepEqual(doc, {
    a: 1,
    b: 'two "quoted" é',
    c: "it's",
    d: "Don't use this",
    e: null,
    f: null,
    g: "-3.5",
    h: "plain text with: colon and (#47)",
    i: "value",
    j: true,
    k: null,
    l: false,
    m: "1.0",
    list: [
      "x",
      2,
      { key: "v", other: "w" },
      { nested: "deep" },
      [1, "two", "three", "a # b"],
      { type: "boolean", default: false },
    ],
    empty_list: [],
    empty_map: {},
    200: { description: "ok" },
    url: "http://localhost:3000",
  });
});

test("yaml: block scalars keep or fold lines and honour chomping; plain scalars continue", () => {
  const doc = parseYaml(`lit: |
  line one
    indented
  line three

fold: >-
  joined by
  spaces

  new paragraph
keep: |+
  kept

next: >
  a
  b
plain: first
  second
  third
after: x
`);
  assert.equal(doc.lit, "line one\n  indented\nline three\n");
  assert.equal(doc.fold, "joined by spaces\nnew paragraph");
  assert.equal(doc.keep, "kept\n\n");
  assert.equal(doc.next, "a b\n");
  assert.equal(doc.plain, "first second third");
  assert.equal(doc.after, "x");
});

test("yaml: errors name the file and line; constructs outside the subset are refused", () => {
  assert.throws(() => parseYaml("a: 1\n  b: 2\n", "x.yaml"), /x\.yaml:2: unexpected indentation/);
  assert.throws(() => parseYaml("a: 1\na: 2\n", "x.yaml"), /x\.yaml:2: duplicate key "a"/);
  assert.throws(
    () => parseYaml("a: &anchor 1\n", "x.yaml"),
    /x\.yaml:1: unsupported YAML construct "&"/,
  );
  assert.throws(() => parseYaml("a: *alias\n", "x.yaml"), /x\.yaml:1: unsupported/);
  assert.throws(() => parseYaml("a: !!str 1\n", "x.yaml"), /x\.yaml:1: unsupported/);
  assert.throws(() => parseYaml('a: "open\n', "x.yaml"), /x\.yaml:1: unterminated/);
  assert.throws(() => parseYaml("a: [1, 2\n", "x.yaml"), /x\.yaml:1: expected , or \]/);
  assert.throws(
    () => parseYaml("a: 1\n- b\n", "x.yaml"),
    /x\.yaml:2: unexpected content after the document/,
  );
  assert.throws(
    () => parseYaml("a:\n\tb: 1\n", "x.yaml"),
    /x\.yaml:2: tabs are not allowed in indentation/,
  );
  assert.throws(
    () => parseYaml("a:\n  plain text on the next line\n", "x.yaml"),
    /x\.yaml:2: expected a key or a sequence item/,
  );
  assert.equal(parseYaml(""), null);
  assert.deepEqual(parseYaml("---\na: 1\n"), { a: 1 });
});

test("yaml: both contracts parse, and their paths and methods match the router", () => {
  const routes = routerRoutes();
  assert.ok(routes.size > 50, "the router was read");
  const api = loadContract(path.join(REPO_ROOT, "api.yaml"));
  const maintenance = loadContract(path.join(REPO_ROOT, "maintenance.yaml"));
  assert.equal(api.doc.openapi, "3.0.3");
  assert.equal(
    api.doc.info.version,
    readVersion(fs.readFileSync(path.join(REPO_ROOT, "Cargo.toml"), "utf8")),
  );
  assert.equal(maintenance.doc.info.version, api.doc.info.version);

  // maintenance.yaml: exactly the /admin/* routes, with exactly their methods.
  const adminRoutes = [...routes.keys()].filter((p) => p.startsWith("/admin/")).sort();
  assert.deepEqual(Object.keys(maintenance.doc.paths).sort(), adminRoutes);
  for (const { path: p, method } of operations(maintenance)) {
    assert.ok(routes.get(p).has(method), `${method.toUpperCase()} ${p} is registered`);
  }
  const maintenanceOps = operations(maintenance).length;
  const adminMethods = adminRoutes.reduce((n, p) => n + routes.get(p).size, 0);
  assert.equal(maintenanceOps, adminMethods, "one operation per registered admin method");

  // api.yaml: every documented path is registered with that method.
  for (const { path: p, method } of operations(api)) {
    assert.ok(routes.has(p), `${p} is registered`);
    assert.ok(routes.get(p).has(method), `${method.toUpperCase()} ${p} is registered`);
  }
  assert.ok(operations(api).length >= 13);
  assert.ok(Object.keys(api.doc.components.schemas).includes("ErrorBody"));
  assert.ok(Object.keys(maintenance.doc.components.schemas).includes("ErrorBody"));
});

// ---------------------------------------------------------------------------
// Generator pieces

test("markdown-lite: paragraphs, lists, inline code, strong, em and links", () => {
  const html = markdownToHtml(
    "One **bold** *em* `a*b` <tag>.\n\nSee [guide](docs/x.md) and [web](https://e.org/).\n- item one\n  continued\n- item two\n\nTail.",
    { repoUrl: "https://github.com/x/y" },
  );
  assert.equal(
    html,
    "<p>One <strong>bold</strong> <em>em</em> <code>a*b</code> &lt;tag&gt;.</p>\n" +
      '<p>See <a href="https://github.com/x/y/blob/summer/docs/x.md">guide</a> and <a href="https://e.org/">web</a>.</p>\n' +
      "<ul><li>item one continued</li><li>item two</li></ul>\n<p>Tail.</p>",
  );
  assert.equal(markdownToHtml(""), "");
});

test("schemas: type labels, property rows, derived examples and $ref resolution", () => {
  const contract = loadContract(fixtureFile);
  const thing = contract.doc.components.schemas.Thing;
  assert.equal(
    typeLabel(contract, { $ref: "#/components/schemas/Thing" }),
    '<a href="#schema-Thing">Thing</a>',
  );
  assert.equal(typeLabel(contract, thing.properties.when), "string (date-time) | null");
  assert.equal(typeLabel(contract, thing.properties.tags), "string[]");
  assert.equal(
    typeLabel(contract, { type: "object", additionalProperties: { type: "integer" } }),
    "map of integer",
  );
  assert.equal(
    typeLabel(contract, { oneOf: [{ type: "string" }, { $ref: "#/components/schemas/Stage" }] }),
    'one of string, <a href="#schema-Stage">Stage</a>',
  );

  const rows = propertyRows(contract, thing);
  assert.deepEqual(
    rows.map((r) => [r.name, r.required]),
    [
      ["name", true],
      ["kind", false],
      ["when", false],
      ["owner", false],
      ["owner.team", false],
      ["tags", false],
      ["stage", false],
      ["parent", false],
    ],
  );
  assert.match(rows[1].notes, /One of <code>big<\/code>, <code>small<\/code>/);
  assert.match(rows[1].notes, /Default <code>&quot;big&quot;<\/code>/);
  assert.equal(rows[6].type, '<a href="#schema-Stage">Stage</a>');
  assert.match(rows[6].notes, /Omitted means draft/);
  assert.match(rows[6].notes, /One of <code>draft<\/code>/);

  // A self-reference through a property is a tree and renders as {}.
  assert.deepEqual(schemaExample(contract, { $ref: "#/components/schemas/Thing" }), {
    name: "string",
    kind: "big",
    when: "2026-01-31T12:00:00Z",
    owner: { team: "string" },
    tags: ["string"],
    stage: "draft",
    parent: {},
  });
  assert.equal(resolveRef(contract, "#/components/schemas/Stage").enum[1], "final");
  assert.throws(
    () => resolveRef(contract, "#/components/schemas/Nope"),
    /fixture\.yaml: \$ref "#\/components\/schemas\/Nope" cannot be resolved/,
  );
  assert.throws(() => resolveRef(contract, "other.yaml#/x"), /not a local reference/);
});

test("a circular $ref through allOf fails naming the schema and the file", () => {
  const file = contractFile(
    "loop.yaml",
    `openapi: "3.0.3"
info: { title: t, version: "1" }
paths:
  /a:
    get:
      responses:
        "200":
          description: ok
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/A"
components:
  schemas:
    A:
      allOf:
        - $ref: "#/components/schemas/B"
    B:
      allOf:
        - $ref: "#/components/schemas/A"
`,
  );
  assert.throws(() => render(file), /circular \$ref via allOf at A in loop\.yaml \(A → B → A\)/);
});

test("two paths or schemas that produce the same anchor fail the build naming both", () => {
  const op = `
    get:
      responses:
        "200":
          description: ok`;
  const file = contractFile(
    "clash.yaml",
    `openapi: "3.0.3"\ninfo: { title: t, version: "1" }\npaths:\n  /x/{id}:${op}\n  /x/id:${op}\n`,
  );
  assert.throws(
    () => render(file),
    /clash\.yaml: anchor "#x-id" is produced by both path "\/x\/\{id\}" and path "\/x\/id"/,
  );
  const dashed = contractFile(
    "dash.yaml",
    `openapi: "3.0.3"\ninfo: { title: t, version: "1" }\npaths:\n  /a-b:${op}\n  /a/b:${op}\n`,
  );
  assert.throws(() => render(dashed), /anchor "#a-b" is produced by both/);
  const schemas = contractFile(
    "schemas.yaml",
    `openapi: "3.0.3"\ninfo: { title: t, version: "1" }\npaths:\n  /a:${op}\ncomponents:\n  schemas:\n    Thing: { type: string }\n    thing: { type: string }\n`,
  );
  // Exact names: Thing and thing are distinct schemas with distinct anchors.
  assert.match(render(schemas).body, /id="schema-Thing"[\s\S]*id="schema-thing"/);
});

test("authentication, permission and scope are read from the contract fields", () => {
  const contract = loadContract(fixtureFile);
  const [get, del] = operations(contract).map((o) => o.op);
  assert.deepEqual(authLabel(contract, get), ["session token"]);
  assert.deepEqual(authLabel(contract, { "x-auth": "none" }), ["none"]);
  assert.deepEqual(authLabel(contract, { "x-auth": "session-or-api-token" }), [
    "session or API token (full or read-only)",
  ]);
  assert.deepEqual(authLabel(contract, { "x-auth": "session-or-api-token-full" }), [
    "session or API token (full)",
  ]);
  assert.throws(() => authLabel(contract, { "x-auth": "magic" }), /unknown x-auth "magic"/);
  // Maintenance operations: session-only, unless a full API token is admitted.
  assert.deepEqual(authLabel(contract, { "x-permission": "manage_users" }), ["session token"]);
  assert.deepEqual(
    authLabel(contract, { "x-permission": "manage_users", "x-api-tokens": "full" }),
    ["session token", "API token (full)"],
  );
  assert.deepEqual(authLabel(contract, {}), ["not stated"]);

  assert.deepEqual(permissionOf(del), {
    permission: "manage_things",
    scope: "per Producer: also its maintainers",
  });
  assert.deepEqual(permissionOf(get), { permission: "not stated", scope: "not stated" });
  assert.deepEqual(permissionOf({ "x-permission": "any-signed-in" }), {
    permission: "any signed-in caller",
    scope: "global",
  });
  assert.deepEqual(permissionOf({ "x-permission": "manage_users" }), {
    permission: "manage_users",
    scope: "global",
  });
  assert.deepEqual(permissionOf({ "x-permission": "manage_users", "x-scope": "consumer" }), {
    permission: "manage_users",
    scope: "per Consumer: also maintainers of the Producer of the same name",
  });
  assert.deepEqual(permissionOf({ description: "Requires the `x` permission." }), {
    permission: "not stated",
    scope: "not stated",
  });
  assert.throws(
    () => permissionOf({ "x-permission": "x", "x-scope": "galaxy" }),
    /unknown x-scope "galaxy"/,
  );
});

test("the real api.yaml classifies exactly the catalog-changing routes as full-token-only", () => {
  const contract = loadContract(path.join(REPO_ROOT, "api.yaml"));
  const byLabel = {};
  for (const { path: p, method, op } of operations(contract)) {
    for (const label of authLabel(contract, op)) (byLabel[label] ||= []).push(`${method} ${p}`);
  }
  // Mirrors write_api_routes_refuse_read_only_tokens in src/lib.rs.
  assert.deepEqual(byLabel["session or API token (full)"].sort(), [
    "get /require",
    "get /require/asyncapi",
    "get /require/grpc",
    "post /provide",
    "post /provide/asyncapi",
    "post /provide/grpc",
    "post /require-bundle",
    "put /catalog/{systemname}",
  ]);
  assert.deepEqual(byLabel.none.sort(), ["get /version", "post /validate"]);
  assert.deepEqual(byLabel["session token"].sort(), [
    "delete /auth/tokens/{id}",
    "get /api/audit/timeline",
    "get /auth/tokens",
    "post /auth/tokens",
  ]);
  assert.deepEqual(byLabel["session or API token (full or read-only)"].sort(), [
    "get /api/sse/updates",
    "get /api/ws/updates",
    "get /endpoint-versions",
    "get /producers/{producername}/versions",
    "get /report",
    "get /report/isolation",
    "get /report/markdown",
  ]);
});

test("every /admin operation shows the permission and scope its router guard enforces", () => {
  const guards = routerGuards();
  assert.ok(guards.size > 100);
  // The promote route is guarded `Authenticated` in the router; the handler
  // itself applies the GA gate (`release_ga`), and the contract documents the
  // gate. Known and deliberate, so it is the one expected difference.
  const exceptions = {
    "post /admin/producers/{name}/versions/{api_type}/{version}/promote": {
      permission: "release_ga",
      scope: "global",
    },
  };
  const blocks = endpointBlocks(built("reference/maintenance.html"));
  assert.equal(blocks.length, guards.size);
  for (const block of blocks) {
    const key = `${block.method} ${block.path}`;
    const guard = guards.get(key);
    assert.ok(guard, `${key} is routed`);
    const expected = exceptions[key] || {
      permission: guard.permission,
      scope: guard.scoped
        ? block.path.startsWith("/admin/consumers/")
          ? "per Consumer: also maintainers of the Producer of the same name"
          : "per Producer: also its maintainers"
        : "global",
    };
    assert.equal(block.permission, expected.permission, `permission of ${key}`);
    assert.equal(block.scope, expected.scope, `scope of ${key}`);
    assert.deepEqual(
      block.chips,
      guard.apiTokens ? ["session token", "API token (full)"] : ["session token"],
      `authentication of ${key}`,
    );
  }
});

test("a recorded example in the contract is shown verbatim instead of the derived placeholder", () => {
  const { body } = render(fixtureFile);
  assert.match(
    body,
    /Example 404 response \(recorded from a live instance\)<\/p>\n<pre class="example"><code class="language-json">\{\n  &quot;name&quot;: &quot;recorded-thing&quot;,\n  &quot;kind&quot;: &quot;small&quot;\n\}<\/code><\/pre>/,
  );
  assert.match(body, /Example 200 response \(placeholder values derived from the schema\)/);
  const api = built("reference/api.html");
  assert.match(api, /Example 202 response \(recorded from a live instance\)/);
  assert.match(api, /&quot;proposed_version&quot;: &quot;1\.1\.0&quot;/);
  assert.match(
    api,
    /Example 200 response \(recorded from a live instance\)<\/p>\n<pre class="example"><code class="language-yaml">openapi: 3\.0\.3/,
  );
});

test("anchors are derived from method and path and stay stable; TOC groups by two segments", () => {
  assert.equal(
    pathSlug("/admin/webhooks/{id}/deliveries/{delivery_id}/redeliver"),
    "admin-webhooks-id-deliveries-delivery-id-redeliver",
  );
  assert.equal(methodAnchor("POST", "/provide"), "post-provide");
  assert.equal(
    methodAnchor("get", "/producers/{producername}/versions"),
    "get-producers-producername-versions",
  );
  assert.equal(pathSlug("/"), "root");
  assert.equal(pathGroup("/admin/producers/{name}/webhooks"), "/admin/producers");
  assert.equal(pathGroup("/provide"), "/provide");
});

test("renderContract: a fixture contract renders parameters, body, responses, security and schemas", () => {
  const page = render(fixtureFile);
  assert.equal(page.track, "reference");
  const { body } = page;
  assert.match(
    body,
    /<span class="chip">fixture\.yaml<\/span> <span class="chip" title="Contract version">version 1\.0<\/span>/,
    "an unquoted 1.0 stays the text 1.0",
  );
  assert.match(
    body,
    /<p class="lede">First paragraph with <code>code<\/code> and <strong>bold<\/strong>\.<\/p>/,
  );
  assert.match(body, /href="https:\/\/github\.com\/x\/y\/blob\/summer\/docs\/guide\.md"/);
  assert.match(body, /<h2 id="things-id"><code>\/things\/\{id\}<\/code><\/h2>/);
  assert.match(
    body,
    /<h3 id="get-things-id"><span class="method method-get">GET<\/span> <code>\/things\/\{id\}<\/code><\/h3>/,
  );
  assert.match(body, /<h3 id="delete-things-id"><span class="method method-delete">DELETE<\/span>/);
  assert.match(body, /<p class="endpoint-summary">42<\/p>/, "a numeric summary is text");
  assert.match(
    body,
    /<p class="endpoint-auth">Authentication: <span class="chip">session token<\/span><\/p>/,
  );
  assert.doesNotMatch(body, /Permission:/, "a non-administrative contract shows no permission");
  const params = body.slice(
    body.indexOf("<h4>Parameters</h4>"),
    body.indexOf("<h4>Responses</h4>"),
  );
  assert.match(
    params,
    /<tr><td><code>id<\/code><\/td><td>path<\/td><td>yes<\/td><td>string<\/td><td><p>The thing\.<\/p><\/td><\/tr>/,
  );
  assert.match(
    params,
    /<tr><td><code>verbose<\/code><\/td><td>query<\/td><td>—<\/td><td>boolean<\/td><td><p>More fields\.<\/p><p>Default <code>false<\/code>\.<\/p><\/td><\/tr>/,
  );
  assert.match(body, /<ul><li>one continued<\/li><li>two<\/li><\/ul>/);
  assert.match(
    body,
    /<tr><td><code>200<\/code><\/td><td><p>The thing\.<\/p><\/td><td><a href="#schema-Thing">Thing<\/a>\n<p class="example-caption">Example 200 response \(placeholder values derived from the schema\)<\/p>\n<pre class="example"><code class="language-json">\{\n  &quot;name&quot;: &quot;string&quot;,/,
  );
  assert.match(
    body,
    /<tr><td><code>401<\/code><\/td><td><p>No valid session\.<\/p><\/td><td>—<\/td><\/tr>/,
  );
  assert.match(
    body,
    /<h2 id="schemas">Schemas<\/h2>\n<h3 id="schema-Thing">Thing<\/h3>\n<p>A thing\.<\/p>/,
  );
  assert.match(
    body,
    /<h3 id="schema-Stage">Stage<\/h3>\n<p>Type: string\. One of <code>draft<\/code>, <code>final<\/code>\.<\/p>/,
  );
  assert.match(body, /<td><code>owner\.team<\/code><\/td><td>string<\/td>/);
  assert.match(
    body,
    /<table class="endpoint-index"><thead><tr><th>Method<\/th><th>Path<\/th><th>Summary<\/th><\/tr><\/thead>[\s\S]*<a href="#get-things-id"><code>\/things\/\{id\}<\/code><\/a><\/td><td>Read a thing<\/td>/,
  );
  assert.match(body, /<a href="api-usage.html">Using the API<\/a>/);
  assert.equal(
    page.toc,
    '<aside class="toc" aria-label="Table of contents">\n<p class="eyebrow">On this page</p><ol>' +
      '<li class="toc-l2"><a href="#endpoints">Endpoints</a></li>' +
      '<li class="toc-l2"><a href="#things-id"><code>/things/{id}</code></a></li>' +
      '<li class="toc-l2"><a href="#schemas">Schemas</a></li></ol>\n</aside>\n',
  );
});

test("an administrative contract shows permission and scope columns and counts the gaps", () => {
  const { body } = render(contractFile("admin.yaml", ADMIN_FIXTURE));
  assert.match(
    body,
    /^<p class="contract-meta">[\s\S]*?<p class="gap-count">1 operation states no permission;/,
  );
  assert.match(
    body,
    /<thead><tr><th>Method<\/th><th>Path<\/th><th>Summary<\/th><th>Permission<\/th><th>Scope<\/th><\/tr><\/thead>/,
  );
  assert.match(
    body,
    /<a href="#post-admin-widgets"><code>\/admin\/widgets<\/code><\/a><\/td><td>Create: widgets<\/td><td><code class="gap">not stated<\/code><\/td><td><code class="gap">not stated<\/code><\/td>/,
  );
  assert.match(
    body,
    /<a href="#delete-admin-widgets-id">[\s\S]*?<td><code>manage_widgets<\/code><\/td><td>global<\/td>/,
  );
  assert.match(
    body,
    /Authentication: <span class="chip">not stated<\/span> · Permission: <code class="gap">not stated<\/code> · Scope: <code class="gap">not stated<\/code>/,
  );
  const { body: clean } = render(
    contractFile(
      "admin2.yaml",
      ADMIN_FIXTURE.replace(
        "      description: Nobody wrote the permission sentence.",
        "      x-permission: manage_widgets\n      x-scope: global\n      description: Declared.",
      ),
    ),
  );
  assert.doesNotMatch(clean, /gap/);
});

test("a contract that cannot be parsed or resolved fails the build with a clear message", () => {
  const site = path.join(tmp, "site");
  fs.mkdirSync(site, { recursive: true });
  fs.writeFileSync(path.join(site, "index.html"), '<!-- page: title="Home" -->\n<p>x</p>');
  const build = (file) =>
    buildSite({
      srcDir: site,
      outDir: path.join(tmp, "out"),
      version: "1.0.0",
      contracts: [{ file, url: "reference/x.html", title: "X", order: 1, summary: "" }],
      arc42: null,
    });
  const broken = contractFile(
    "broken.yaml",
    "openapi: 3\ninfo:\n  title: t\n  version: 1\npaths:\n  /a:\n    get: 1\n   bad: 2\n",
  );
  assert.throws(() => build(broken), /broken\.yaml:8: unexpected indentation/);
  const dangling = contractFile(
    "dangling.yaml",
    'openapi: 3\ninfo:\n  title: t\n  version: 1\npaths:\n  /a:\n    get:\n      responses:\n        "200":\n          description: ok\n          content:\n            application/json:\n              schema:\n                $ref: "#/components/schemas/Missing"\n',
  );
  assert.throws(
    () => build(dangling),
    /dangling\.yaml: \$ref "#\/components\/schemas\/Missing" cannot be resolved/,
  );
  const noPaths = contractFile("nopaths.yaml", "info:\n  title: t\n  version: 1\n");
  assert.throws(() => build(noPaths), /nopaths\.yaml: not an OpenAPI document/);
  assert.throws(
    () => build(path.join(tmp, "absent.yaml")),
    /absent\.yaml: cannot read the contract/,
  );
});

// ---------------------------------------------------------------------------
// The real site build

test("the real site build renders both references into the Reference track", () => {
  assert.equal(DEFAULT_CONTRACTS.length, 2);
  assert.ok(
    !fs.existsSync(path.join(REPO_ROOT, "site/reference/api.html")),
    "generated only into the output",
  );

  const api = built("reference/api.html");
  assert.match(api, /<title>REST API — Sanshain<\/title>/);
  assert.match(api, /<h1>REST API<\/h1>/);
  assert.match(api, /<p class="eyebrow track-eyebrow">Reference<\/p>/);
  const contractVersion = fs
    .readFileSync(path.join(REPO_ROOT, "api.yaml"), "utf8")
    .match(/^ {2}version: "([^"]+)"/m)[1];
  assert.match(api, new RegExp(`version ${contractVersion.replace(/\./g, "\\.")}</span>`));
  assert.doesNotMatch(api, /gap-count/, "api.yaml has no permission gaps to count");
  // The representative path: anchor, chip, summary, security, body schema link and example, responses.
  assert.match(
    api,
    /<h3 id="post-provide"><span class="method method-post">POST<\/span> <code>\/provide<\/code><a class="anchor" href="#post-provide"/,
  );
  assert.match(api, /Provide an OpenAPI specification under its declared version/);
  assert.match(api, /<span class="chip">session or API token \(full\)<\/span>/);
  assert.match(api, /<a href="#schema-ProvidePayload">ProvidePayload<\/a>/);
  assert.match(
    api,
    /Example request body \(placeholder values derived from the schema\)<\/p>\n<pre class="example"><code class="language-json">\{\n  &quot;producername&quot;: &quot;string&quot;,[\s\S]*?&quot;stability&quot;: &quot;snapshot&quot;/,
  );
  assert.match(api, /<tr><td><code>409<\/code><\/td><td><p>Rejected by the version rules\./);
  assert.match(api, /<h3 id="get-require">/);
  assert.match(
    api,
    /<tr><td><code>consumername<\/code><\/td><td>query<\/td><td>yes<\/td><td>string<\/td>/,
  );
  assert.match(api, /<span class="chip">none<\/span>[\s\S]*<h3 id="get-version">/);
  assert.match(api, /<h3 id="schema-ErrorBody">ErrorBody<a class="anchor"/);
  assert.match(api, /<td><code>proposed_version<\/code><\/td><td>string<\/td><td>—<\/td>/);
  assert.match(api, /<div class="table-wrap"><table class="endpoint-index">/);

  // The TOC lists paths, not methods, grouped by the first two segments.
  const toc = api.slice(api.indexOf('<aside class="toc"'), api.indexOf("</aside>"));
  assert.match(toc, /<li class="toc-l2"><a href="#provide"><code>\/provide<\/code><\/a><\/li>/);
  assert.doesNotMatch(toc, /toc-l3|#post-provide/);
  assert.match(
    toc,
    /<li class="toc-group"><details><summary><code>\/auth\/tokens<\/code> <span class="toc-count">2<\/span><\/summary><ol><li class="toc-l2"><a href="#auth-tokens">/,
  );

  const maintenance = built("reference/maintenance.html");
  assert.match(maintenance, /<title>Maintenance API — Sanshain<\/title>/);
  assert.match(maintenance, /<h3 id="post-admin-webhooks-id-deliveries-delivery-id-redeliver">/);
  assert.match(
    maintenance,
    /<thead><tr><th>Method<\/th><th>Path<\/th><th>Summary<\/th><th>Permission<\/th><th>Scope<\/th><\/tr><\/thead>/,
  );
  assert.match(
    maintenance,
    /href="https:\/\/paxel\.github\.io\/sanshain-service\/operator\/roles\.html"/,
  );
  const mtoc = maintenance.slice(
    maintenance.indexOf('<aside class="toc"'),
    maintenance.indexOf("</aside>"),
  );
  assert.match(
    mtoc,
    /<summary><code>\/admin\/producers<\/code> <span class="toc-count">\d+<\/span>/,
  );
  assert.ok((mtoc.match(/<details>/g) || []).length >= 8, "collapsible groups");

  const nav = api.slice(api.indexOf('id="sidebar"'), api.indexOf("</nav>"));
  const labels = ["Reference", "Using the API", "REST API", "Maintenance API"].map((l) =>
    nav.indexOf(l),
  );
  assert.ok(labels.every((i) => i >= 0));
  assert.deepEqual(
    labels,
    [...labels].sort((a, b) => a - b),
  );
  assert.match(nav, /<a href="\.\.\/reference\/api\.html" aria-current="page">REST API<\/a>/);

  const usage = built("reference/api-usage.html");
  assert.match(usage, /<h1>Using the API<\/h1>/);
  assert.match(usage, /href="api\.html#post-provide"/);
  assert.match(usage, /href="api\.html#schema-ErrorBody"/);
});

test("the longest admin path sits in a heading with the wrap rule the stylesheet carries", () => {
  const maintenance = built("reference/maintenance.html");
  const paths = [...maintenance.matchAll(/<h2 id="[^"]+"><code>([^<]+)<\/code>/g)].map((m) => m[1]);
  const longest = paths.reduce((a, b) => (b.length > a.length ? b : a), "");
  assert.ok(longest.length >= 60, `longest path is ${longest}`);
  assert.match(
    maintenance,
    new RegExp(
      `<h3 id="[^"]+"><span class="method[^"]*">\\w+</span> <code>${longest.replace(/[{}]/g, "\\$&")}</code>`,
    ),
  );
  const css = built("assets/site.css");
  assert.match(
    css,
    /h2 code,\nh3 code,\n\.endpoint code,\n\.endpoint-index td code,\n\.toc code \{\n {2}overflow-wrap: anywhere;\n\}/,
  );
  assert.match(css, /@media print \{[^}]*:root:root,\n {2}:root:root\[data-theme="dark"\] \{/);
});

test("the search index lists endpoints by path and summary and leaves the example JSON out", () => {
  const index = JSON.parse(built("search-index.json"));
  const apiEntry = index.find((p) => p.url === "reference/api.html");
  assert.equal(apiEntry.track, "Reference");
  const provide = apiEntry.headings.find((h) => h.id === "post-provide");
  assert.equal(provide.text, "POST /provide");
  assert.match(provide.body, /^Provide an OpenAPI specification under its declared version/);
  assert.doesNotMatch(provide.body, /"producername": "string"|"stability": "snapshot"/);
  assert.match(provide.body, /Example request body/, "the caption stays, the JSON goes");
  const maintenanceEntry = index.find((p) => p.url === "reference/maintenance.html");
  const redeliver = maintenanceEntry.headings.find(
    (h) => h.id === "post-admin-webhooks-id-deliveries-delivery-id-redeliver",
  );
  assert.equal(redeliver.text, "POST /admin/webhooks/{id}/deliveries/{delivery_id}/redeliver");
  assert.match(redeliver.body, /redeliver/i);
  const urls = index.map((p) => p.url);
  assert.ok(urls.indexOf("reference/api-usage.html") < urls.indexOf("reference/api.html"));
  assert.ok(urls.indexOf("reference/api.html") < urls.indexOf("reference/maintenance.html"));
});
