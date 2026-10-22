// The structure view of the spec viewer (yaml.html): the server's tree of an
// endpoint (parameters, request body, responses, AsyncAPI messages, every
// $ref inlined) drawn as nested <details>, each node with its type, flags,
// description and the version that last changed it. In full-spec mode the
// endpoints arrive without children and load the first time one is opened.

// Nodes that open by themselves when an endpoint is shown unfolded: the
// sections, not the members inside them.
const STRUCTURE_OPEN_KINDS = new Set(["endpoint", "group", "response", "body", "message"]);

const structureFlagStyles = {
  required: "bg-rose-500/20 text-rose-300",
  deprecated: "bg-amber-500/20 text-amber-300 line-through",
  nullable: "bg-slate-700 text-slate-300",
  read_only: "bg-sky-500/20 text-sky-300",
  write_only: "bg-sky-500/20 text-sky-300",
};

function structureJson(value) {
  return JSON.stringify(value, null, 2);
}

// The server's JSON objects arrive with sorted keys; put an example's keys
// back in the order the schema declares its members, so a copied body reads
// like the spec. Keys the tree does not know keep their place after them.
function structureOrdered(value, node) {
  if (!node) return value;
  const children = node.children || [];
  if (Array.isArray(value)) {
    const items = children.find((c) => c.kind === "items");
    return value.map((v) => structureOrdered(v, items));
  }
  if (value === null || typeof value !== "object") return value;
  const members = children.filter((c) => c.kind === "member" || c.kind === "cycle");
  const variants = children.find((c) => c.kind === "variants");
  if (variants) members.push(...(variants.children || []).flatMap((v) => v.children || []));
  const ordered = {};
  for (const member of members) {
    if (Object.hasOwn(value, member.key) && !Object.hasOwn(ordered, member.key)) {
      ordered[member.key] = structureOrdered(value[member.key], member);
    }
  }
  for (const [k, v] of Object.entries(value)) {
    if (!Object.hasOwn(ordered, k)) ordered[k] = v;
  }
  return ordered;
}

function structureChip(text, cls, title) {
  const t = title ? ` title="${escapeAttr(title)}"` : "";
  return `<span class="text-[10px] font-semibold px-1.5 py-0.5 rounded-full ${cls}"${t}>${escapeHtml(text)}</span>`;
}

// The row of a node: name, type, flags and version chips.
function structureSummaryHtml(node, viewedVersion) {
  const parts = [];
  const nameCls =
    node.kind === "endpoint"
      ? "font-mono font-bold text-slate-100"
      : node.kind === "group" || node.kind === "response" || node.kind === "message"
        ? "font-semibold text-slate-100"
        : "font-mono text-slate-100";
  parts.push(`<span class="${nameCls}">${escapeHtml(node.name)}</span>`);
  if (node.type) {
    parts.push(`<span class="font-mono text-indigo-300">${escapeHtml(node.type)}</span>`);
  }
  for (const flag of Object.keys(structureFlagStyles)) {
    if (node[flag]) {
      parts.push(structureChip(flag.replace("_", "-"), structureFlagStyles[flag]));
    }
  }
  if (node.origin) {
    parts.push(structureChip(`from ${node.origin}`, "bg-slate-700 text-slate-300"));
  }
  if (node.discriminator) {
    parts.push(
      structureChip(`by ${node.discriminator}`, "bg-slate-700 text-slate-300", "discriminator"),
    );
  }
  if (node.changed_in) {
    const isNew = node.changed_in === viewedVersion;
    parts.push(
      structureChip(
        node.changed_in,
        isNew ? "bg-amber-400/20 text-amber-300" : "bg-indigo-500/20 text-indigo-300",
        `Last changed in ${node.changed_in}`,
      ),
    );
  }
  if (node.inside) {
    parts.push(
      structureChip(
        `inside: ${node.inside}`,
        "bg-amber-400/10 text-amber-200",
        `Something below changed in ${node.inside}`,
      ),
    );
  }
  if (node.summary && node.kind === "endpoint") {
    parts.push(`<span class="text-slate-400">${escapeHtml(node.summary)}</span>`);
  }
  return `<span class="inline-flex flex-wrap items-center gap-x-2 gap-y-1">${parts.join("")}</span>`;
}

// What a node says besides its row: description, default, example, allowed
// values and constraints.
function structureDetailsHtml(node) {
  const lines = [];
  if (node.description && !(node.kind === "endpoint" && node.description === node.summary)) {
    lines.push(
      `<div class="text-slate-300 whitespace-pre-line" data-structure-description>${escapeHtml(node.description)}</div>`,
    );
  }
  const value = (label, v) =>
    `<div class="text-slate-400"><span class="text-slate-500">${label}</span> <code class="font-mono text-emerald-300">${escapeHtml(typeof v === "string" ? JSON.stringify(v) : structureJson(v))}</code></div>`;
  if (node.default !== undefined) lines.push(value("default", node.default));
  if (node.example !== undefined && node.kind !== "body") {
    lines.push(value("example", node.example));
  }
  if (node.enum && node.enum.length) {
    lines.push(
      `<div class="text-slate-400"><span class="text-slate-500">one of</span> ${node.enum
        .map(
          (v) => `<code class="font-mono text-emerald-300">${escapeHtml(JSON.stringify(v))}</code>`,
        )
        .join(", ")}</div>`,
    );
  }
  if (node.constraints && node.constraints.length) {
    lines.push(
      `<div class="text-slate-400"><span class="text-slate-500">constraints</span> ${escapeHtml(node.constraints.join(", "))}</div>`,
    );
  }
  return lines.length ? `<div class="space-y-1 pl-5 pb-1">${lines.join("")}</div>` : "";
}

// A body's example as a folded child with a Copy button.
function structureExampleHtml(node, examples) {
  if (node.kind !== "body" || node.example === undefined) return "";
  const index = examples.push(structureJson(structureOrdered(node.example, node))) - 1;
  const source = node.example_generated ? "built from the schema" : "from the spec";
  return `<details class="pl-5" data-structure-example>
      <summary class="cursor-pointer py-0.5 text-slate-300 font-semibold">Example <span class="text-slate-500 font-normal">${source}</span></summary>
      <div class="relative my-1">
        <button type="button" class="absolute right-2 top-2 text-[11px] px-2 py-0.5 rounded bg-slate-700 text-slate-200 hover:bg-slate-600" data-structure-copy="${index}">Copy</button>
        <pre class="font-mono text-xs bg-slate-950 border border-slate-800 rounded-md p-3 overflow-x-auto text-emerald-200">${escapeHtml(examples[index])}</pre>
      </div>
    </details>`;
}

function structureNodeHtml(node, ctx, depth) {
  const summary = structureSummaryHtml(node, ctx.version);
  const details = structureDetailsHtml(node);
  const example = structureExampleHtml(node, ctx.examples);
  const children = (node.children || []).map((c) => structureNodeHtml(c, ctx, depth + 1)).join("");
  const lazy = node.children_omitted
    ? ` data-structure-lazy data-path="${escapeAttr(node.path || "")}" data-method="${escapeAttr(node.method || "")}"`
    : "";
  if (!details && !example && !children && !lazy) {
    return `<div class="py-0.5 pl-5" data-structure-node="${escapeAttr(node.key)}">${summary}</div>`;
  }
  const open = ctx.unfold && STRUCTURE_OPEN_KINDS.has(node.kind) ? " open" : "";
  const body = lazy
    ? `<div class="pl-4 text-slate-500 italic" data-structure-children>Loading…</div>`
    : `<div class="pl-4 border-l border-slate-800 ml-1.5" data-structure-children>${example}${children}</div>`;
  return `<details${open}${lazy} data-structure-node="${escapeAttr(node.key)}">
      <summary class="cursor-pointer py-0.5">${summary}</summary>
      ${details}${body}
    </details>`;
}

// Draw `view` (the server's answer) into `container`. `loadEndpoint(path,
// method)` answers one endpoint's view, for endpoints that arrive folded.
function renderStructureView(container, view, { unfold, loadEndpoint, copy }) {
  const ctx = { version: view.version, examples: [], unfold };
  const endpoints = view.endpoints || [];
  const tree = endpoints.map((e) => structureNodeHtml(e, ctx, 0)).join("");
  container.innerHTML = `<div class="font-sans text-[13px] leading-relaxed whitespace-normal select-text" data-structure-root>
      <div class="flex items-center gap-2 mb-3">
        <button type="button" class="text-xs px-2.5 py-1 rounded-md bg-slate-800 text-slate-200 hover:bg-slate-700" data-structure-expand>Expand all</button>
        <button type="button" class="text-xs px-2.5 py-1 rounded-md bg-slate-800 text-slate-200 hover:bg-slate-700" data-structure-collapse>Collapse all</button>
        <span class="text-xs text-slate-500">Version chips name the version that last changed a node.</span>
      </div>
      ${tree || '<div class="text-slate-400 italic">No endpoints in this version.</div>'}
    </div>`;
  const root = container.querySelector("[data-structure-root]");
  let expandAll = false;

  const fill = async (details) => {
    if (!details.hasAttribute("data-structure-lazy")) return;
    details.removeAttribute("data-structure-lazy");
    const slot = details.querySelector(":scope > [data-structure-children]");
    try {
      const one = await loadEndpoint(details.dataset.path, details.dataset.method);
      const node = (one.endpoints || [])[0];
      const inner = { version: view.version, examples: ctx.examples, unfold: true };
      slot.className = "pl-4 border-l border-slate-800 ml-1.5";
      slot.innerHTML = node
        ? (node.children || []).map((c) => structureNodeHtml(c, inner, 1)).join("")
        : "";
      if (expandAll) slot.querySelectorAll("details").forEach((d) => (d.open = true));
    } catch (e) {
      details.setAttribute("data-structure-lazy", "");
      slot.textContent = `Could not load this endpoint: ${e.message}`;
    }
  };

  // `toggle` does not bubble; listen in the capture phase.
  root.addEventListener(
    "toggle",
    (e) => {
      if (e.target.open) fill(e.target);
    },
    true,
  );
  root.addEventListener("click", (e) => {
    const copyButton = e.target.closest("[data-structure-copy]");
    if (copyButton) {
      copy(ctx.examples[Number(copyButton.dataset.structureCopy)]);
      return;
    }
    if (e.target.closest("[data-structure-expand]")) {
      expandAll = true;
      root.querySelectorAll("details").forEach((d) => (d.open = true));
    } else if (e.target.closest("[data-structure-collapse]")) {
      expandAll = false;
      root.querySelectorAll("details").forEach((d) => (d.open = false));
    }
  });
}
