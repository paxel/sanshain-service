// A YAML subset reader for the two contract files (api.yaml, maintenance.yaml).
//
// Supported: block mappings and sequences (spaces only; a tab in leading
// whitespace is an error), plain scalars with continuation lines, double- and
// single-quoted scalars, literal (|) and folded (>) block scalars with
// chomping indicators, single-line flow collections ([a, b], { type: string }),
// comments (a `#` preceded by whitespace ends a plain scalar), quoted keys.
// Plain scalars resolve `true/false/null/~` in their YAML 1.2 core spellings
// (true, True, TRUE …) and integers; everything else, including anything with
// a dot such as `1.0`, stays a string, because a version is a string and the
// contracts carry no floats.
//
// Unsupported, refused with file:line: anchors, aliases, tags, multi-document
// streams, complex keys, explicit block indentation indicators, multi-line
// flow collections, and a plain multi-line value that starts on the line
// after its key (`key:` followed by indented text; use `|` or `>`).

class YamlError extends Error {
  constructor(file, line, message) {
    super(`${file}:${line}: ${message}`);
    this.name = "YamlError";
  }
}

const KEY = /^(?:"((?:[^"\\]|\\.)*)"|'((?:[^']|'')*)'|([^\s"'#&*!|>%@`{}[\],][^#]*?))\s*:(?:\s+|$)/;

function plainScalar(text) {
  if (text === "" || text === "~" || /^(null|Null|NULL)$/.test(text)) return null;
  if (/^(true|True|TRUE)$/.test(text)) return true;
  if (/^(false|False|FALSE)$/.test(text)) return false;
  if (/^[-+]?(0|[1-9]\d*)$/.test(text)) return Number(text);
  return text;
}

const ESCAPES = { n: "\n", t: "\t", r: "\r", '"': '"', "\\": "\\", "/": "/", 0: "\0" };

function unescapeDouble(inner, fail) {
  return inner.replace(/\\(u([0-9a-fA-F]{4})|.)/g, (whole, ch, hex) => {
    if (hex) return String.fromCharCode(parseInt(hex, 16));
    if (ch in ESCAPES) return ESCAPES[ch];
    return fail(`unknown escape ${whole} in double-quoted string`);
  });
}

/** A plain scalar ends at the first ` #`; a quote inside it is just a character. */
function stripComment(text) {
  const at = text.search(/\s#/);
  return (at >= 0 ? text.slice(0, at) : text).trimEnd();
}

/** A flow collection ends at the first ` #` outside its quoted strings. */
function stripFlowComment(text) {
  let quote = null;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (quote) {
      if (c === "\\" && quote === '"') i++;
      else if (c === quote) quote = null;
    } else if (c === '"' || c === "'") quote = c;
    else if (c === "#" && i > 0 && /\s/.test(text[i - 1])) return text.slice(0, i).trimEnd();
  }
  return text.trimEnd();
}

export function parseYaml(source, file = "yaml") {
  const raw = source.split(/\r?\n/);
  // Pre-pass: every line with its indent; blank and comment lines are kept
  // (block scalars need them) but flagged so the structural walk skips them.
  const lines = raw.map((text, i) => {
    const indent = text.search(/\S|$/);
    const trimmed = text.trim();
    return { no: i + 1, text, indent, blank: trimmed === "" || trimmed.startsWith("#") };
  });
  const fail = (line, message) => {
    throw new YamlError(file, line.no, message);
  };
  for (const line of lines) {
    if (line.text.slice(0, line.indent).includes("\t")) {
      fail(line, "tabs are not allowed in indentation");
    }
  }

  let pos = 0;
  const next = () => {
    while (pos < lines.length && lines[pos].blank) pos++;
    return pos < lines.length ? lines[pos] : null;
  };

  // Scalar parsing, shared by mapping values and sequence items.

  function flowValue(text, line) {
    let i = 0;
    const ws = () => {
      while (i < text.length && /\s/.test(text[i])) i++;
    };
    const scalar = () => {
      ws();
      if (text[i] === '"') {
        const m = /^"((?:[^"\\]|\\.)*)"/.exec(text.slice(i));
        if (!m) fail(line, "unterminated double-quoted string");
        i += m[0].length;
        return unescapeDouble(m[1], (msg) => fail(line, msg));
      }
      if (text[i] === "'") {
        const m = /^'((?:[^']|'')*)'/.exec(text.slice(i));
        if (!m) fail(line, "unterminated single-quoted string");
        i += m[0].length;
        return m[1].replace(/''/g, "'");
      }
      const start = i;
      while (i < text.length && !/[,\]}]/.test(text[i])) i++;
      const word = text.slice(start, i).trim();
      if (word.includes(": ")) fail(line, "flow mappings inside flow values must be {…}");
      return plainScalar(word);
    };
    const value = () => {
      ws();
      if (text[i] === "[") {
        i++;
        const out = [];
        ws();
        if (text[i] === "]") {
          i++;
          return out;
        }
        for (;;) {
          out.push(value());
          ws();
          if (text[i] === ",") {
            i++;
            ws();
            if (text[i] === "]") {
              i++;
              return out;
            }
            continue;
          }
          if (text[i] === "]") {
            i++;
            return out;
          }
          fail(line, "expected , or ] in flow sequence");
        }
      }
      if (text[i] === "{") {
        i++;
        const out = {};
        ws();
        if (text[i] === "}") {
          i++;
          return out;
        }
        for (;;) {
          ws();
          const keyStart = i;
          while (i < text.length && text[i] !== ":") i++;
          if (i >= text.length) fail(line, "expected key: value in flow mapping");
          const key = text
            .slice(keyStart, i)
            .trim()
            .replace(/^"(.*)"$|^'(.*)'$/, "$1$2");
          i++;
          out[key] = value();
          ws();
          if (text[i] === ",") {
            i++;
            ws();
            if (text[i] === "}") {
              i++;
              return out;
            }
            continue;
          }
          if (text[i] === "}") {
            i++;
            return out;
          }
          fail(line, "expected , or } in flow mapping");
        }
      }
      return scalar();
    };
    const result = value();
    ws();
    if (i < text.length) fail(line, `unexpected text after flow value: "${text.slice(i)}"`);
    return result;
  }

  /** Block scalar: `|` or `>` with optional chomping; content lines deeper than parentIndent. */
  function blockScalar(indicator, line, parentIndent) {
    const m = /^([|>])([+-]?)(\d*)$|^([|>])(\d*)([+-]?)$/.exec(indicator);
    if (!m) fail(line, `unsupported block scalar header "${indicator}"`);
    const folded = (m[1] || m[4]) === ">";
    const chomp = m[2] || m[6] || "";
    if (m[3] || m[5]) fail(line, "explicit block indentation indicators are not supported");
    pos++;
    const content = [];
    let contentIndent = -1;
    while (pos < lines.length) {
      const l = lines[pos];
      if (l.text.trim() === "") {
        content.push("");
        pos++;
        continue;
      }
      if (contentIndent < 0) {
        if (l.indent <= parentIndent) break;
        contentIndent = l.indent;
      }
      if (l.indent < contentIndent) break;
      content.push(l.text.slice(contentIndent));
      pos++;
    }
    // Trailing blank lines belong to chomping, not to the content.
    let trailing = 0;
    while (content.length && content[content.length - 1] === "") {
      content.pop();
      trailing++;
    }
    let body;
    if (folded) {
      body = "";
      for (let i = 0; i < content.length; i++) {
        const cur = content[i];
        if (i === 0) body = cur;
        else if (cur === "") body += "\n";
        else if (content[i - 1] === "" || /^\s/.test(cur) || /^\s/.test(content[i - 1]))
          body += (content[i - 1] === "" ? "" : "\n") + cur;
        else body += " " + cur;
      }
    } else body = content.join("\n");
    if (!content.length) return chomp === "+" ? "\n".repeat(trailing) : "";
    if (chomp === "-") return body;
    if (chomp === "+") return body + "\n".repeat(trailing + 1);
    return body + "\n";
  }

  /** A scalar on one line, possibly continued on deeper plain lines. */
  function inlineScalar(text, line, parentIndent) {
    const t = text.trim();
    if (t === "") {
      pos++;
      return null;
    }
    if (t[0] === "|" || t[0] === ">") return blockScalar(t, line, parentIndent);
    if (t[0] === "[" || t[0] === "{") {
      pos++;
      return flowValue(stripFlowComment(t), line);
    }
    if (t[0] === '"') {
      const m = /^"((?:[^"\\]|\\.)*)"\s*(#.*)?$/.exec(t);
      if (!m) fail(line, "unterminated or trailing text after double-quoted string");
      pos++;
      return unescapeDouble(m[1], (msg) => fail(line, msg));
    }
    if (t[0] === "'") {
      const m = /^'((?:[^']|'')*)'\s*(#.*)?$/.exec(t);
      if (!m) fail(line, "unterminated or trailing text after single-quoted string");
      pos++;
      return m[1].replace(/''/g, "'");
    }
    if (/^[&*!%@`]/.test(t))
      fail(line, `unsupported YAML construct "${t[0]}" (anchor, alias or tag)`);
    let value = stripComment(t);
    pos++;
    // Plain multi-line scalar: following non-blank lines deeper than the
    // parent that are not themselves a key or a sequence item.
    while (pos < lines.length) {
      const l = lines[pos];
      if (l.blank) {
        if (l.text.trim() === "") {
          const after = next();
          if (
            after &&
            after.indent > parentIndent &&
            !KEY.test(after.text.trim()) &&
            !/^-(\s|$)/.test(after.text.trim())
          ) {
            value += "\n";
            continue;
          }
        }
        break;
      }
      if (l.indent <= parentIndent) break;
      const lt = l.text.trim();
      if (KEY.test(lt) || /^-(\s|$)/.test(lt)) break;
      value += (value.endsWith("\n") ? "" : " ") + stripComment(lt);
      pos++;
    }
    return plainScalar(value);
  }

  // Block collections.

  function block(indent) {
    const line = next();
    if (!line) return null;
    const text = line.text.slice(line.indent);
    if (/^-(\s|$)/.test(text)) return sequence(line.indent);
    if (KEY.test(text)) return mapping(line.indent);
    return fail(line, `expected a key or a sequence item at indent ${indent}`);
  }

  function value(afterKey, line, keyIndent) {
    if (afterKey.trim() !== "" && !afterKey.trim().startsWith("#")) {
      return inlineScalar(afterKey, line, keyIndent);
    }
    pos++;
    const n = next();
    if (!n) return null;
    if (n.indent > keyIndent) return block(n.indent);
    // A sequence may sit at the key's own indent (`key:\n- a`).
    if (n.indent === keyIndent && /^-(\s|$)/.test(n.text.trim())) return sequence(n.indent);
    return null;
  }

  function mapping(indent) {
    const out = {};
    for (;;) {
      const line = next();
      if (!line || line.indent < indent) break;
      if (line.indent > indent) fail(line, "unexpected indentation");
      const text = line.text.slice(indent);
      if (/^-(\s|$)/.test(text)) break;
      const m = KEY.exec(text);
      if (!m) fail(line, `expected "key: value", got "${text.slice(0, 40)}"`);
      const key =
        m[1] !== undefined
          ? unescapeDouble(m[1], (msg) => fail(line, msg))
          : m[2] !== undefined
            ? m[2].replace(/''/g, "'")
            : m[3].trim();
      if (key in out) fail(line, `duplicate key "${key}"`);
      out[key] = value(text.slice(m[0].length), line, indent);
    }
    return out;
  }

  function sequence(indent) {
    const out = [];
    for (;;) {
      const line = next();
      if (!line || line.indent < indent) break;
      if (line.indent > indent) fail(line, "unexpected indentation");
      const text = line.text.slice(indent);
      if (!/^-(\s|$)/.test(text)) break;
      const rest = text.slice(1);
      const inner = rest.trim();
      if (inner === "" || inner.startsWith("#")) {
        pos++;
        const n = next();
        out.push(n && n.indent > indent ? block(n.indent) : null);
        continue;
      }
      // `- key: value` starts a mapping whose indent is the first key's column.
      if (KEY.test(inner) && !/^[[{]/.test(inner)) {
        const innerIndent = indent + 1 + (rest.length - rest.trimStart().length);
        // Rewrite the line in place so the mapping reader sees a plain key line.
        lines[pos] = { ...line, text: " ".repeat(innerIndent) + inner, indent: innerIndent };
        out.push(mapping(innerIndent));
        continue;
      }
      out.push(inlineScalar(inner, line, indent));
    }
    return out;
  }

  // Document.
  const first = next();
  if (!first) return null;
  if (first.text.trim() === "---") pos++;
  const doc = block(next() ? next().indent : 0);
  const rest = next();
  if (rest) fail(rest, `unexpected content after the document: "${rest.text.trim().slice(0, 40)}"`);
  return doc;
}

export { YamlError };
