// REST reference pages generated from the OpenAPI contracts (api.yaml and
// maintenance.yaml): one page per contract, a section per path, a block per
// method, and a Schemas section at the end. The pages are plain fragments
// like authored ones, so the shell, search index and link check treat them
// the same way; only the table of contents is built here, grouped by path,
// because two hundred flat entries do not fit a phone.

import fs from "node:fs";
import path from "node:path";
import { escapeHtml } from "./html.js";
import { parseYaml } from "./yaml.js";

const METHODS = ["get", "post", "put", "patch", "delete", "head", "options"];
const PLACEHOLDER = "placeholder values derived from the schema";

// ---------------------------------------------------------------------------
// Loading and $ref resolution

/** `summary` and `description` are prose: a number or boolean there becomes its text. */
function normaliseProse(node) {
  if (Array.isArray(node)) {
    node.forEach(normaliseProse);
  } else if (node && typeof node === "object") {
    for (const [key, value] of Object.entries(node)) {
      if (
        (key === "summary" || key === "description") &&
        ["number", "boolean"].includes(typeof value)
      ) {
        node[key] = String(value);
      } else normaliseProse(value);
    }
  }
}

export function loadContract(file) {
  let source;
  try {
    source = fs.readFileSync(file, "utf8");
  } catch (err) {
    throw new Error(`${path.basename(file)}: cannot read the contract: ${err.message}`);
  }
  const name = path.basename(file);
  const doc = parseYaml(source, name);
  if (!doc || typeof doc !== "object" || !doc.paths || typeof doc.paths !== "object") {
    throw new Error(`${name}: not an OpenAPI document (no "paths" mapping)`);
  }
  if (!doc.info || typeof doc.info.title !== "string" || doc.info.version === undefined) {
    throw new Error(`${name}: "info" must carry a title and a version`);
  }
  normaliseProse(doc);
  return { doc, name };
}

/** Follows a local `#/components/...` reference; throws naming the contract and the ref. */
export function resolveRef(contract, ref) {
  if (typeof ref !== "string" || !ref.startsWith("#/")) {
    throw new Error(`${contract.name}: $ref "${ref}" is not a local reference`);
  }
  let node = contract.doc;
  for (const part of ref.slice(2).split("/")) {
    const key = part.replace(/~1/g, "/").replace(/~0/g, "~");
    if (node === null || typeof node !== "object" || !(key in node)) {
      throw new Error(`${contract.name}: $ref "${ref}" cannot be resolved`);
    }
    node = node[key];
  }
  return node;
}

const deref = (contract, node) =>
  node && typeof node === "object" && node.$ref ? resolveRef(contract, node.$ref) : node;

function refName(ref) {
  return ref.slice(ref.lastIndexOf("/") + 1);
}

// ---------------------------------------------------------------------------
// Markdown-lite: the contracts' descriptions use paragraphs, "- " lists,
// `code`, **strong**, *em* and [links](url). Relative links point at files in
// the repository and become GitHub links.

function inlineMarkdown(text, repoUrl) {
  const link = (_, label, url) =>
    `<a href="${/^https?:/i.test(url) ? url : `${repoUrl}/blob/summer/${url}`}">${label}</a>`;
  return text
    .split(/(`[^`]*`)/)
    .map((part, i) =>
      i % 2
        ? `<code>${escapeHtml(part.slice(1, -1))}</code>`
        : escapeHtml(part)
            .replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>")
            .replace(/(^|[\s(])\*([^*\s][^*]*?)\*(?=[\s).,;:!?]|$)/g, "$1<em>$2</em>")
            .replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, link),
    )
    .join("");
}

export function markdownToHtml(text, { repoUrl = "" } = {}) {
  if (!text) return "";
  const out = [];
  let para = [];
  let items = null;
  const flushPara = () => {
    if (para.length) out.push(`<p>${inlineMarkdown(para.join(" "), repoUrl)}</p>`);
    para = [];
  };
  const flushList = () => {
    if (items)
      out.push(`<ul>${items.map((i) => `<li>${inlineMarkdown(i, repoUrl)}</li>`).join("")}</ul>`);
    items = null;
  };
  for (const raw of String(text).split("\n")) {
    const line = raw.trim();
    if (line === "") {
      flushPara();
      flushList();
    } else if (/^- /.test(line)) {
      flushPara();
      items = items || [];
      items.push(line.slice(2));
    } else if (items && /^\s/.test(raw)) {
      items[items.length - 1] += ` ${line}`;
    } else {
      flushList();
      para.push(line);
    }
  }
  flushPara();
  flushList();
  return out.join("\n");
}

// ---------------------------------------------------------------------------
// Schemas: type labels, property tables and derived examples

/** Schemas are keyed by their exact component name; the id is escaped where it is used. */
const schemaId = (name) => `schema-${name}`;

function schemaLink(name) {
  return `<a href="#${escapeHtml(schemaId(name))}">${escapeHtml(name)}</a>`;
}

/** A short type label; `$ref`s become links to the Schemas section. */
export function typeLabel(contract, schema) {
  if (!schema || typeof schema !== "object") return "any";
  if (schema.$ref) return schemaLink(refName(schema.$ref));
  if (schema.allOf) return schema.allOf.map((s) => typeLabel(contract, s)).join(" &amp; ");
  if (schema.oneOf) return `one of ${schema.oneOf.map((s) => typeLabel(contract, s)).join(", ")}`;
  if (schema.anyOf) return `any of ${schema.anyOf.map((s) => typeLabel(contract, s)).join(", ")}`;
  let label;
  if (schema.type === "array") label = `${typeLabel(contract, schema.items)}[]`;
  else if (schema.type === "object" && schema.additionalProperties && !schema.properties) {
    label =
      schema.additionalProperties === true
        ? "object"
        : `map of ${typeLabel(contract, schema.additionalProperties)}`;
  } else if (schema.type) label = escapeHtml(schema.type);
  else if (schema.properties) label = "object";
  else label = "any";
  if (schema.format) label += ` (${escapeHtml(schema.format)})`;
  if (schema.nullable) label += " | null";
  return label;
}

/** Enum, default and description of a schema, for a table cell. */
function constraints(contract, schema, description) {
  const notes = [];
  const text = description ?? schema?.description;
  if (text) notes.push(markdownToHtml(text, contract));
  if (schema?.enum) {
    notes.push(
      `<p>One of ${schema.enum.map((v) => `<code>${escapeHtml(v)}</code>`).join(", ")}.</p>`,
    );
  }
  if (schema?.default !== undefined) {
    notes.push(`<p>Default <code>${escapeHtml(JSON.stringify(schema.default))}</code>.</p>`);
  }
  return notes.join("");
}

/**
 * Flattens `allOf` into one object schema. `seen` holds the `$ref`s on the
 * current chain: meeting one again is a cycle, which no page can render.
 */
function mergeAllOf(contract, schema, seen = []) {
  const merged = { properties: {}, required: [] };
  for (const part of schema.allOf) {
    let chain = seen;
    if (part.$ref) {
      if (seen.includes(part.$ref)) {
        throw new Error(
          `circular $ref via allOf at ${refName(part.$ref)} in ${contract.name} (${[...seen, part.$ref].map(refName).join(" → ")})`,
        );
      }
      chain = [...seen, part.$ref];
    }
    const resolved = deref(contract, part);
    const inner = resolved.allOf ? mergeAllOf(contract, resolved, chain) : resolved;
    Object.assign(merged.properties, inner.properties || {});
    merged.required.push(...(inner.required || []));
    if (inner.type) merged.type = inner.type;
    if (inner.enum) merged.enum = inner.enum;
  }
  return merged;
}

/** Rows (name, type, required, notes) for an object schema, nested inline objects flattened as a.b and a[].b. */
export function propertyRows(contract, schema, prefix = "", seen = []) {
  const rows = [];
  const base = schema.allOf ? mergeAllOf(contract, schema, seen) : schema;
  const required = new Set(base.required || []);
  for (const [name, prop] of Object.entries(base.properties || {})) {
    const full = `${prefix}${name}`;
    // A property wrapping one `$ref` in allOf to add a description keeps the ref as its type.
    const shown =
      prop.allOf && prop.allOf.length === 1 && prop.allOf[0].$ref ? prop.allOf[0] : prop;
    rows.push({
      name: full,
      type: typeLabel(contract, shown),
      required: required.has(name),
      notes: constraints(
        contract,
        shown.$ref ? resolveRef(contract, shown.$ref) : prop,
        prop.description,
      ),
    });
    if (prop.type === "object" && prop.properties)
      rows.push(...propertyRows(contract, prop, `${full}.`, seen));
    if (prop.type === "array" && prop.items && prop.items.properties) {
      rows.push(...propertyRows(contract, prop.items, `${full}[].`, seen));
    }
  }
  return rows;
}

function renderPropertyTable(contract, schema, seen) {
  const rows = propertyRows(contract, schema, "", seen);
  if (!rows.length) return "";
  const body = rows
    .map(
      (r) =>
        `<tr><td><code>${escapeHtml(r.name)}</code></td><td>${r.type}</td>` +
        `<td>${r.required ? "yes" : "—"}</td><td>${r.notes}</td></tr>`,
    )
    .join("\n");
  return (
    `<table class="schema-table"><thead><tr><th>Field</th><th>Type</th><th>Required</th>` +
    `<th>Description</th></tr></thead><tbody>\n${body}\n</tbody></table>`
  );
}

const FORMAT_EXAMPLES = {
  "date-time": "2026-01-31T12:00:00Z",
  date: "2026-01-31",
  uri: "https://example.org/",
  int64: 1,
  int32: 1,
  binary: "<binary>",
};

/**
 * A JSON value shaped by the schema: defaults, first enum value, format
 * hints. A `$ref` met again through properties renders as `{}` (a tree is a
 * legitimate shape); met again through `allOf` it is an error.
 */
export function schemaExample(contract, schema, seen = []) {
  if (!schema || typeof schema !== "object") return null;
  if (schema.$ref) {
    if (seen.includes(schema.$ref)) return {};
    return schemaExample(contract, resolveRef(contract, schema.$ref), [...seen, schema.$ref]);
  }
  if (schema.example !== undefined) return schema.example;
  if (schema.allOf) return schemaExample(contract, mergeAllOf(contract, schema, seen), seen);
  if (schema.oneOf || schema.anyOf)
    return schemaExample(contract, (schema.oneOf || schema.anyOf)[0], seen);
  if (schema.default !== undefined) return schema.default;
  if (schema.enum) return schema.enum[0];
  switch (schema.type) {
    case "object": {
      const out = {};
      for (const [name, prop] of Object.entries(schema.properties || {})) {
        out[name] = schemaExample(contract, prop, seen);
      }
      if (
        !schema.properties &&
        schema.additionalProperties &&
        schema.additionalProperties !== true
      ) {
        out.key = schemaExample(contract, schema.additionalProperties, seen);
      }
      return out;
    }
    case "array":
      return [schemaExample(contract, schema.items, seen)];
    case "string":
      return FORMAT_EXAMPLES[schema.format] ?? "string";
    case "integer":
    case "number":
      return FORMAT_EXAMPLES[schema.format] ?? 0;
    case "boolean":
      return false;
    default:
      return schema.nullable
        ? null
        : schema.properties
          ? schemaExample(contract, { ...schema, type: "object" }, seen)
          : null;
  }
}

/** Generated examples carry class="example" so the search index can leave them out. */
const RECORDED = "recorded from a live instance";

function exampleBlock(contract, schema, caption, seen) {
  const json = JSON.stringify(schemaExample(contract, schema, seen), null, 2);
  return (
    `<p class="example-caption">${caption} (${PLACEHOLDER})</p>\n` +
    `<pre class="example"><code class="language-json">${escapeHtml(json)}</code></pre>`
  );
}

/**
 * The example a media-type object carries in the contract (`example`), as
 * recorded from a live instance: an object or array is shown as JSON, a
 * string as it is. Preferred over the derived placeholder.
 */
function recordedExampleBlock(media, content, caption) {
  const { example } = content;
  if (example === undefined) return null;
  const text = typeof example === "string" ? example : JSON.stringify(example, null, 2);
  const lang = /json/.test(media) || typeof example !== "string" ? "json" : "yaml";
  return (
    `<p class="example-caption">${caption} (${RECORDED})</p>\n` +
    `<pre class="example"><code class="language-${lang}">${escapeHtml(text)}</code></pre>`
  );
}

// ---------------------------------------------------------------------------
// Authentication, permission and scope

const AUTH = {
  none: "none",
  session: "session token",
  apiFull: "API token (full)",
  eitherFull: "session or API token (full)",
  eitherAny: "session or API token (full or read-only)",
};

const X_AUTH = {
  none: [AUTH.none],
  session: [AUTH.session],
  "session-or-api-token": [AUTH.eitherAny],
  "session-or-api-token-full": [AUTH.eitherFull],
};

/**
 * What a call must carry, read from the contract's `x-auth` field (api.yaml:
 * none, session, session-or-api-token, session-or-api-token-full). The
 * maintenance contract is session-only unless the operation carries
 * `x-api-tokens: full`, which admits a full API token beside the session. A
 * build-time test in src/lib.rs keeps both fields equal to the router guards.
 */
export function authLabel(contract, op) {
  const auth = op["x-auth"];
  if (auth !== undefined) {
    const labels = X_AUTH[auth];
    if (!labels) throw new Error(`${contract.name}: unknown x-auth "${auth}"`);
    return labels;
  }
  if (op["x-api-tokens"] === "full") return [AUTH.session, AUTH.apiFull];
  if (op["x-permission"] !== undefined) return [AUTH.session];
  return [NOT_STATED];
}

export const NOT_STATED = "not stated";

const X_SCOPE = {
  global: "global",
  producer: "per Producer: also its maintainers",
  consumer: "per Consumer: also maintainers of the Producer of the same name",
};

/**
 * The permission an administrative operation needs and its scope, from the
 * contract's `x-permission` (a permission's wire name, or `any-signed-in`)
 * and `x-scope` (global, producer, consumer). An operation without the
 * fields is shown as a gap.
 */
export function permissionOf(op) {
  const permission = op["x-permission"];
  if (permission === undefined) return { permission: NOT_STATED, scope: NOT_STATED };
  const scope = X_SCOPE[op["x-scope"] ?? "global"];
  if (!scope) throw new Error(`unknown x-scope "${op["x-scope"]}"`);
  return {
    permission: permission === "any-signed-in" ? "any signed-in caller" : permission,
    scope,
  };
}

const permissionCell = (text) =>
  text === NOT_STATED
    ? `<code class="gap">${NOT_STATED}</code>`
    : `<code>${escapeHtml(text)}</code>`;

// ---------------------------------------------------------------------------
// Rendering

export function pathSlug(p) {
  return (
    p
      .replace(/[{}]/g, "")
      .replace(/[^a-zA-Z0-9]+/g, "-")
      .replace(/^-|-$/g, "")
      .toLowerCase() || "root"
  );
}

export const methodAnchor = (method, p) => `${method.toLowerCase()}-${pathSlug(p)}`;

function methodChip(method) {
  const m = method.toLowerCase();
  return `<span class="method method-${m}">${m.toUpperCase()}</span>`;
}

function renderParameters(contract, params) {
  if (!params || !params.length) return "";
  const rows = params
    .map((raw) => {
      const p = deref(contract, raw);
      return (
        `<tr><td><code>${escapeHtml(p.name)}</code></td><td>${escapeHtml(p.in)}</td>` +
        `<td>${p.required ? "yes" : "—"}</td><td>${typeLabel(contract, p.schema)}</td>` +
        `<td>${constraints(contract, p.schema, p.description)}</td></tr>`
      );
    })
    .join("\n");
  return (
    `<h4>Parameters</h4>\n<table class="params-table"><thead><tr><th>Name</th><th>In</th>` +
    `<th>Required</th><th>Type</th><th>Description</th></tr></thead><tbody>\n${rows}\n</tbody></table>`
  );
}

function renderBodySchema(contract, schema) {
  if (schema.$ref) return `<p>Schema: ${schemaLink(refName(schema.$ref))}.</p>`;
  const table = renderPropertyTable(contract, schema);
  return table || `<p>Schema: ${typeLabel(contract, schema)}.</p>`;
}

function renderRequestBody(contract, body) {
  if (!body) return "";
  const resolved = deref(contract, body);
  const parts = [`<h4>Request body${resolved.required ? "" : " (optional)"}</h4>`];
  if (resolved.description) parts.push(markdownToHtml(resolved.description, contract));
  for (const [media, content] of Object.entries(resolved.content || {})) {
    parts.push(`<p><code>${escapeHtml(media)}</code></p>`);
    if (!content.schema) continue;
    parts.push(renderBodySchema(contract, content.schema));
    const recorded = recordedExampleBlock(media, content, "Example request body");
    if (recorded) parts.push(recorded);
    else if (/json/.test(media)) {
      parts.push(exampleBlock(contract, content.schema, "Example request body"));
    }
  }
  return parts.join("\n");
}

function renderResponses(contract, responses) {
  const entries = Object.entries(responses || {});
  if (!entries.length) return "";
  const rows = entries.map(([status, raw]) => {
    const r = deref(contract, raw);
    const notes = [markdownToHtml(r.description || "", contract)];
    for (const [name, header] of Object.entries(r.headers || {})) {
      const h = deref(contract, header);
      const text = h.description
        ? `: ${inlineMarkdown(h.description.trim().replace(/\s+/g, " "), contract.repoUrl)}`
        : "";
      notes.push(`<p>Header <code>${escapeHtml(name)}</code>${text}</p>`);
    }
    const bodies = [];
    for (const [media, content] of Object.entries(r.content || {})) {
      let cell = content.schema ? typeLabel(contract, content.schema) : "—";
      if (!/json/.test(media)) cell += ` <small>(${escapeHtml(media)})</small>`;
      const caption = `Example ${escapeHtml(status)} response`;
      const recorded = recordedExampleBlock(media, content, caption);
      if (recorded) cell += `\n${recorded}`;
      else if (content.schema && /json/.test(media) && /^2/.test(status)) {
        cell += `\n${exampleBlock(contract, content.schema, caption)}`;
      }
      bodies.push(cell);
    }
    return (
      `<tr><td><code>${escapeHtml(status)}</code></td><td>${notes.join("")}</td>` +
      `<td>${bodies.join("<br>") || "—"}</td></tr>`
    );
  });
  return (
    `<h4>Responses</h4>\n<table class="responses-table"><thead><tr><th>Status</th><th>Description</th>` +
    `<th>Body</th></tr></thead><tbody>\n${rows.join("\n")}\n</tbody></table>`
  );
}

function renderAuth(contract, op, administrative) {
  const chips = authLabel(contract, op).map((l) => `<span class="chip">${escapeHtml(l)}</span>`);
  let meta = `Authentication: ${chips.join(" ")}`;
  if (administrative) {
    const { permission, scope } = permissionOf(op);
    meta += ` · Permission: ${permissionCell(permission)} · Scope: ${scope === NOT_STATED ? permissionCell(scope) : escapeHtml(scope)}`;
  }
  return `<p class="endpoint-auth">${meta}</p>`;
}

function renderOperation(contract, p, method, op, pathItem, administrative) {
  const params = [...(pathItem.parameters || []), ...(op.parameters || [])];
  return [
    `<section class="endpoint">`,
    `<h3 id="${methodAnchor(method, p)}">${methodChip(method)} <code>${escapeHtml(p)}</code></h3>`,
    op.summary
      ? `<p class="endpoint-summary">${inlineMarkdown(op.summary, contract.repoUrl)}</p>`
      : "",
    renderAuth(contract, op, administrative),
    markdownToHtml(op.description, contract),
    renderParameters(contract, params),
    renderRequestBody(contract, op.requestBody),
    renderResponses(contract, op.responses),
    `</section>`,
  ]
    .filter(Boolean)
    .join("\n");
}

function renderSchemas(contract) {
  const schemas = contract.doc.components?.schemas || {};
  const names = Object.keys(schemas);
  if (!names.length) return "";
  const sections = names.map((name) => {
    const schema = schemas[name];
    const seen = [`#/components/schemas/${name}`];
    const parts = [`<h3 id="${escapeHtml(schemaId(name))}">${escapeHtml(name)}</h3>`];
    if (schema.description) parts.push(markdownToHtml(schema.description, contract));
    const table = renderPropertyTable(contract, schema, seen);
    if (table) parts.push(table);
    else
      parts.push(
        `<p>Type: ${typeLabel(contract, schema)}.${schema.enum ? ` One of ${schema.enum.map((v) => `<code>${escapeHtml(v)}</code>`).join(", ")}.` : ""}</p>`,
      );
    if (schema.type === "object" || schema.properties || schema.allOf || schema.oneOf) {
      parts.push(exampleBlock(contract, schema, "Example", seen));
    }
    return parts.join("\n");
  });
  return `<h2 id="schemas">Schemas</h2>\n${sections.join("\n")}`;
}

/** Every (path, method, operation) of the contract, in document order. */
export function operations(contract) {
  const out = [];
  for (const [p, item] of Object.entries(contract.doc.paths)) {
    if (!item || typeof item !== "object")
      throw new Error(`${contract.name}: path ${p} has no operations`);
    for (const method of METHODS) {
      if (item[method]) out.push({ path: p, method, op: item[method], pathItem: item });
    }
  }
  return out;
}

/** The first two segments of a path: the group a table of contents entry belongs to. */
export function pathGroup(p) {
  const segments = p.split("/").filter(Boolean);
  return `/${segments.slice(0, 2).join("/")}`;
}

/**
 * Table of contents: Endpoints, one entry per path grouped by its first two
 * segments (a group of several paths collapses), Schemas. The method and
 * schema anchors stay in the page for links and search.
 */
function renderToc(paths) {
  const groups = new Map();
  for (const p of paths) {
    const g = pathGroup(p);
    if (!groups.has(g)) groups.set(g, []);
    groups.get(g).push(p);
  }
  const entry = (p) =>
    `<li class="toc-l2"><a href="#${pathSlug(p)}"><code>${escapeHtml(p)}</code></a></li>`;
  const items = [`<li class="toc-l2"><a href="#endpoints">Endpoints</a></li>`];
  for (const [g, members] of groups) {
    if (members.length === 1) items.push(entry(members[0]));
    else {
      items.push(
        `<li class="toc-group"><details><summary><code>${escapeHtml(g)}</code> <span class="toc-count">${members.length}</span></summary>` +
          `<ol>${members.map(entry).join("")}</ol></details></li>`,
      );
    }
  }
  items.push(`<li class="toc-l2"><a href="#schemas">Schemas</a></li>`);
  return `<aside class="toc" aria-label="Table of contents">\n<p class="eyebrow">On this page</p><ol>${items.join("")}</ol>\n</aside>\n`;
}

/** Every anchor the page emits must be unique: two paths or schemas that slug alike are an error. */
function checkAnchors(contract, paths, ops, schemaNames) {
  const owners = new Map();
  const claim = (id, owner) => {
    if (owners.has(id)) {
      throw new Error(
        `${contract.name}: anchor "#${id}" is produced by both ${owners.get(id)} and ${owner}`,
      );
    }
    owners.set(id, owner);
  };
  claim("endpoints", "the Endpoints heading");
  claim("schemas", "the Schemas heading");
  for (const p of paths) claim(pathSlug(p), `path "${p}"`);
  for (const o of ops)
    claim(methodAnchor(o.method, o.path), `${o.method.toUpperCase()} "${o.path}"`);
  for (const name of schemaNames) claim(schemaId(name), `schema "${name}"`);
}

/**
 * Renders one contract into a page object ({ title, track, order, summary,
 * url, body, toc }) the site build treats like an authored page.
 */
export function renderContract(
  contract,
  { title, url, order, summary, repoUrl = "", usageUrl = "" },
) {
  const ctx = { ...contract, repoUrl };
  const { doc } = contract;
  const pathNames = Object.keys(doc.paths);
  const administrative = pathNames.every((p) => p.startsWith("/admin/"));
  const ops = operations(ctx);
  checkAnchors(ctx, pathNames, ops, Object.keys(doc.components?.schemas || {}));

  const permissions = ops.map((o) => (administrative ? permissionOf(o.op) : null));
  const indexHead =
    `<tr><th>Method</th><th>Path</th><th>Summary</th>` +
    (administrative ? `<th>Permission</th><th>Scope</th></tr>` : `</tr>`);
  const index = ops
    .map((o, i) => {
      const row =
        `<tr><td>${methodChip(o.method)}</td><td><a href="#${methodAnchor(o.method, o.path)}"><code>${escapeHtml(o.path)}</code></a></td>` +
        `<td>${o.op.summary ? inlineMarkdown(o.op.summary, repoUrl) : ""}</td>`;
      if (!administrative) return `${row}</tr>`;
      const { permission, scope } = permissions[i];
      return `${row}<td>${permissionCell(permission)}</td><td>${scope === NOT_STATED ? permissionCell(scope) : escapeHtml(scope)}</td></tr>`;
    })
    .join("\n");
  const gaps = permissions.filter((p) => p && p.permission === NOT_STATED).length;
  const gapLine = gaps
    ? `<p class="gap-count">${gaps} operation${gaps === 1 ? "" : "s"} state${gaps === 1 ? "s" : ""} no permission; the description of each has no "Requires the … permission" sentence.</p>`
    : "";

  const paths = pathNames.map((p) => {
    const blocks = ops
      .filter((o) => o.path === p)
      .map((o) => renderOperation(ctx, p, o.method, o.op, doc.paths[p], administrative));
    return `<h2 id="${pathSlug(p)}"><code>${escapeHtml(p)}</code></h2>\n${blocks.join("\n")}`;
  });
  const intro = markdownToHtml(doc.info.description || "", ctx).replace(/^<p>/, '<p class="lede">');
  const usage = usageUrl
    ? `<p>Tokens, scopes, sessions and error shapes are explained in <a href="${usageUrl}">Using the API</a>.</p>`
    : "";
  const body = [
    `<p class="contract-meta"><span class="chip">${escapeHtml(contract.name)}</span> ` +
      `<span class="chip" title="Contract version">version ${escapeHtml(doc.info.version)}</span></p>`,
    intro,
    usage,
    gapLine,
    `<h2 id="endpoints">Endpoints</h2>`,
    `<table class="endpoint-index"><thead>${indexHead}</thead><tbody>\n${index}\n</tbody></table>`,
    ...paths,
    renderSchemas(ctx),
  ]
    .filter(Boolean)
    .join("\n");
  return { title, track: "reference", order, summary, url, body, toc: renderToc(pathNames) };
}

/** Loads and renders each contract description ({ file, title, url, order, summary }). */
export function contractPages(contracts, options = {}) {
  return contracts.map((c) => renderContract(loadContract(c.file), { ...options, ...c }));
}
