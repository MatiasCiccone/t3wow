const IDENT = /^[A-Za-z_][A-Za-z0-9_]*$/;
const RESERVED = new Set(
  "and break do else elseif end false for function if in local nil not or repeat return then true until while goto".split(" "),
);

export function luaString(value) {
  let out = '"';
  for (const ch of String(value)) {
    const code = ch.codePointAt(0);
    if (ch === "\\") out += "\\\\";
    else if (ch === '"') out += '\\"';
    else if (ch === "\n") out += "\\n";
    else if (ch === "\r") out += "\\r";
    else if (code < 32 || code === 127) out += "\\" + String(code).padStart(3, "0");
    else out += ch;
  }
  return out + '"';
}

/** JS value -> Lua 5.1 literal. Arrays become sequences; undefined/null fields are dropped. */
export function toLua(value) {
  if (value === null || value === undefined) return "nil";
  if (typeof value === "boolean") return value ? "true" : "false";
  if (typeof value === "number") return Number.isFinite(value) ? String(value) : "0";
  if (typeof value === "string") return luaString(value);
  if (Array.isArray(value)) return "{" + value.map(toLua).join(",") + "}";
  const parts = [];
  for (const [key, field] of Object.entries(value)) {
    if (field === null || field === undefined) continue;
    const name = IDENT.test(key) && !RESERVED.has(key) ? key : `[${luaString(key)}]`;
    parts.push(`${name}=${toLua(field)}`);
  }
  return "{" + parts.join(",") + "}";
}

const CODE = "|cffc8c8a0";
const HEAD = "|cffffd100";

/**
 * Markdown to WoW FontString markup. `|` starts an escape sequence in WoW text, so
 * literal pipes are doubled before any color codes are added.
 */
export function wowText(markdown, maxLength = 6000) {
  let text = String(markdown ?? "").replace(/\r\n?/g, "\n");
  if (text.length > maxLength) text = text.slice(0, maxLength) + "\n…";
  text = text.replace(/\|/g, "||");
  const lines = [];
  let inFence = false;
  for (const line of text.split("\n")) {
    if (/^\s*```/.test(line)) {
      inFence = !inFence;
      continue;
    }
    if (inFence) {
      lines.push(`${CODE}${line}|r`);
      continue;
    }
    let l = line
      .replace(/!\[([^\]]*)\]\([^)]*\)/g, "[$1]")
      .replace(/\[([^\]]+)\]\(([^)]+)\)/g, "$1")
      .replace(/\*\*([^*]+)\*\*/g, "$1")
      .replace(/__([^_]+)__/g, "$1")
      .replace(/`([^`]+)`/g, `${CODE}$1|r`)
      .replace(/^(\s*)[-*] /, "$1• ");
    const heading = /^#{1,6}\s+(.*)$/.exec(l);
    if (heading) l = `${HEAD}${heading[1]}|r`;
    lines.push(l);
  }
  return lines.join("\n").replace(/\n{3,}/g, "\n\n").trim();
}
