// Browser-safe directive validation; no Node runtime dependencies.
const INJECTION_CHARS = /[\s;{}#"'\r\n]/;

export type DirectiveContext = "server" | "location";

/**
 * Which nginx directives users may add, per context. Anything not listed is
 * rejected before it can reach a rendered file (`nginx -t` is the final net).
 * File-referencing / module-loading / static-serving / TLS directives are
 * deliberately excluded — see the design spec. Extend these sets to allow more.
 */
export const DIRECTIVE_ALLOWLIST: Record<DirectiveContext, ReadonlySet<string>> = {
  location: new Set([
    "proxy_set_header",
    "add_header",
    "proxy_read_timeout",
    "proxy_send_timeout",
    "proxy_connect_timeout",
    "proxy_http_version",
    "proxy_buffering",
    "client_max_body_size",
    "rewrite",
    "return",
    "proxy_redirect",
    "proxy_hide_header",
    "proxy_pass_header",
    "expires",
  ]),
  server: new Set([
    "add_header",
    "client_max_body_size",
    "server_tokens",
    "proxy_read_timeout",
    "proxy_send_timeout",
    "proxy_connect_timeout",
    "keepalive_timeout",
    "gzip",
  ]),
};

/**
 * Directives whose LAST argument is a header value that may legitimately
 * contain spaces or `;` (e.g. HSTS `max-age=…; includeSubDomains`). The
 * renderer double-quotes such values; validation therefore allows spaces/`;`
 * in the value but still forbids `"`/CR/LF so it can never break out of the
 * quotes. The header NAME and any trailing flag token stay plain tokens.
 */
export const HEADER_DIRECTIVES: ReadonlySet<string> = new Set(["proxy_set_header", "add_header"]);

/**
 * Chars that must never appear even inside a quoted header value. A `"` would
 * close the rendered quote; a backslash could escape the renderer's own
 * closing quote (`"...\"` → nginx treats `\"` as a literal quote, so the string
 * runs on and injects config); CR/LF would split the line. Forbidding all four
 * makes the double-quoted render provably safe.
 */
const HEADER_VALUE_FORBIDDEN = /["\\\r\n]/;

/**
 * Validates one directive (`name` + tokenized `args`) for a context. Used by
 * BOTH the structured form editors and the free-form parser, so there is a
 * single security-critical code path. Throws a human-readable `Error`.
 */
export function assertValidDirective(name: string, args: string[], context: DirectiveContext): void {
  if (typeof name !== "string" || name.length === 0) {
    throw new Error("directive name must not be empty");
  }
  const allow = DIRECTIVE_ALLOWLIST[context];
  if (!allow.has(name)) {
    const hint = name === "resolver" || name === "resolver_timeout"
      ? " resolver는 직접 설정할 수 없습니다. 대상 주소가 바뀌는 경우 「실행 중에 이름 확인」을 켜고, DNS 서버는 NWUI_TARGET_DNS_SERVERS로 지정하세요."
      : "";
    throw new Error(`Directive not allowed in ${context} context: "${name}"${hint}`);
  }
  if (!Array.isArray(args) || args.length === 0) {
    throw new Error(`Directive "${name}" needs at least one argument`);
  }

  if (HEADER_DIRECTIVES.has(name)) {
    const headerName = args[0] ?? "";
    if (headerName.length === 0 || INJECTION_CHARS.test(headerName)) {
      throw new Error(`Invalid header name for "${name}": "${headerName}"`);
    }
    if (args.length < 2) {
      throw new Error(`Directive "${name}" needs a value`);
    }
    const value = args[1] ?? "";
    if (HEADER_VALUE_FORBIDDEN.test(value)) {
      throw new Error(`Invalid header value (no quotes or newlines): "${value}"`);
    }
    // Any further tokens (e.g. add_header's optional `always`) are plain tokens.
    for (const t of args.slice(2)) {
      if (typeof t !== "string" || t.length === 0 || INJECTION_CHARS.test(t)) {
        throw new Error(`Invalid argument for "${name}": "${t}"`);
      }
    }
    return;
  }

  for (const t of args) {
    if (typeof t !== "string" || t.length === 0 || INJECTION_CHARS.test(t)) {
      throw new Error(`Invalid argument for "${name}": "${t}"`);
    }
  }
}

/**
 * Tokenizes ONE directive line (no newlines), honoring single/double quotes.
 * Rejects an unquoted `;` that is not the sole trailing terminator (would
 * smuggle a second directive), unquoted `{`/`}` (blocks), and `#` (comments).
 * Throws on unterminated quotes. Returns the token list (quotes stripped).
 */
function tokenizeDirectiveLine(line: string): string[] {
  const tokens: string[] = [];
  const n = line.length;
  let i = 0;
  while (i < n) {
    while (i < n && /\s/.test(line[i]!)) i++;
    if (i >= n) break;
    const c = line[i]!;
    if (c === "#") {
      throw new Error(`Comments (#) are not allowed in a directive line: "${line}"`);
    }
    if (c === "{" || c === "}") {
      throw new Error(`Blocks ({ }) are not allowed in a directive line: "${line}"`);
    }
    if (c === ";") {
      if (line.slice(i + 1).trim().length > 0) {
        throw new Error(`Only one directive per line (unexpected ";"): "${line}"`);
      }
      break; // sole trailing terminator
    }
    if (c === '"' || c === "'") {
      i++;
      let val = "";
      while (i < n && line[i] !== c) {
        val += line[i];
        i++;
      }
      if (i >= n) {
        throw new Error(`Unterminated quoted string: "${line}"`);
      }
      i++; // closing quote
      tokens.push(val);
    } else {
      let val = "";
      while (i < n && !/[\s;{}#"']/.test(line[i]!)) {
        val += line[i];
        i++;
      }
      tokens.push(val);
    }
  }
  return tokens;
}

/**
 * Parses the advanced free-form textarea (one directive per line) into
 * validated `{ name, args }[]`. Blank lines and whole-line `#` comments are
 * skipped. Each parsed directive is checked with `assertValidDirective`, so
 * free-form and structured input share identical validation.
 */
export function parseFreeformDirectives(
  text: string,
  context: DirectiveContext,
): { name: string; args: string[] }[] {
  const out: { name: string; args: string[] }[] = [];
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (line.length === 0 || line.startsWith("#")) continue;
    const tokens = tokenizeDirectiveLine(line);
    if (tokens.length === 0) continue;
    const [name, ...args] = tokens;
    assertValidDirective(name!, args, context);
    out.push({ name: name!, args });
  }
  return out;
}
