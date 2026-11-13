import type { Directive } from "./types";
import { assertValidDirective, parseFreeformDirectives, type DirectiveContext } from "./directive-validation";

/**
 * The DirectiveEditor's UI-shaped state and the pure converters between it and
 * a validated `Directive[]`. Lives in @repo/presentation (which has a test runner) so
 * these — the security-adjacent glue that feeds validated directives to the
 * renderer — can be unit-tested; apps/web has no test runner. The module is
 * PURE (only `./directive-validation` + `./types`, no node), so apps/web imports it via
 * the `@repo/presentation/nginx/directive-state` subpath without pulling apply.ts/control.ts
 * (which use node:fs / node:child_process) into the client bundle.
 *
 * Two functions share the structured-slot mapping and MUST stay in lockstep:
 *  - `tryHoistDirective`  (stored Directive -> which state field)
 *  - `stateToDirectives`  (state fields -> ordered Directive[])
 * `directive-state.test.ts` pins their agreement + the order-preservation and
 * quoting round-trip guarantees, so adding/renaming a slot can't silently drift.
 */

export interface HeaderRow {
  name: string;
  value: string;
}

export interface RewriteRow {
  from: string;
  to: string;
  flag: string; // "" | "break" | "last" | "redirect" | "permanent"
}

export interface DirectiveEditorState {
  headers: HeaderRow[]; // proxy_set_header (location context)
  timeouts: { read: string; send: string; connect: string }; // proxy_*_timeout, e.g. "60s"
  bodySize: string; // client_max_body_size, e.g. "10m"
  addHeaders: HeaderRow[]; // add_header
  rewrites: RewriteRow[]; // rewrite (location context)
  httpVersion: string; // "" | "1.0" | "1.1" (location context)
  buffering: string; // "" | "on" | "off" (location context)
  advanced: string; // free-form allowlisted directives, one per line
}

export function emptyDirectiveState(): DirectiveEditorState {
  return {
    headers: [],
    timeouts: { read: "", send: "", connect: "" },
    bodySize: "",
    addHeaders: [],
    rewrites: [],
    httpVersion: "",
    buffering: "",
    advanced: "",
  };
}

/**
 * 새 location 을 만들 때 미리 채워지는 헤더들.
 *
 * `Host` 만으로는 upstream 이 **모든 요청을 프록시의 IP 에서 온 것으로 본다** —
 * 접속 로그·차단·감사가 전부 무의미해진다. 리버스 프록시를 세우면 거의 항상 함께
 * 넣는 세 줄이라 기본값으로 둔다. 필요 없으면 폼에서 지우면 된다.
 *
 * `$proxy_add_x_forwarded_for` 는 기존 X-Forwarded-For 뒤에 client IP 를 덧붙인다
 * (덮어쓰지 않는다) — 프록시가 여러 단이어도 체인이 보존된다.
 */
export const DEFAULT_LOCATION_HEADERS: readonly HeaderRow[] = [
  { name: "Host", value: "$host" },
  { name: "X-Real-IP", value: "$remote_addr" },
  { name: "X-Forwarded-For", value: "$proxy_add_x_forwarded_for" },
  { name: "X-Forwarded-Proto", value: "$scheme" },
];

/** 위 기본 헤더로 미리 채워진 새 location 편집 상태. */
export function seededLocationDirectiveState(): DirectiveEditorState {
  return { ...emptyDirectiveState(), headers: DEFAULT_LOCATION_HEADERS.map((h) => ({ ...h })) };
}

const REWRITE_FLAGS = new Set(["break", "last", "redirect", "permanent"]);

// Mirrors `renderHeaderValue` (render.ts) EXACTLY: a single quote (') is
// significant to the nginx tokenizer just like a double quote, so it must
// trigger quoting here too — otherwise a saved header value like `it's` would
// be re-emitted unquoted and fail to re-parse on the next edit ("Unterminated
// quoted string"). Backslash/quote are escaped defensively (validated header
// values contain neither, but keeping the two predicates in lockstep prevents
// a round-trip mismatch).
function quoteArg(a: string): string {
  return a.length === 0 || /[\s;{}#'"]/.test(a) ? `"${a.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"` : a;
}

/** Reconstructs a free-form line for a directive that has no structured slot. */
function directiveToLine(d: Directive): string {
  return `${d.name} ${d.args.map(quoteArg).join(" ")};`;
}

// ---------------------------------------------------------------------------
// state -> Directive[]  (used by the server action on submit)
// ---------------------------------------------------------------------------

/**
 * Converts one DirectiveEditor's UI state into an ordered, validated
 * `Directive[]`. Order matches the renderer's expectation (headers → timeouts
 * → body size → add_header → rewrite → http_version/buffering → advanced).
 * Empty structured rows/fields are skipped. Throws (human-readable) on the
 * first invalid entry — same `assertValidDirective` the free-form parser uses.
 */
export function stateToDirectives(state: DirectiveEditorState, context: DirectiveContext): Directive[] {
  const dirs: Directive[] = [];

  for (const h of state.headers ?? []) {
    const name = h.name.trim();
    // Header values are preserved as-is (only the emptiness check trims): a
    // value may legitimately carry meaningful leading/trailing spaces.
    if (!name && !h.value.trim()) continue;
    dirs.push({ name: "proxy_set_header", args: [name, h.value] });
  }

  const t = state.timeouts ?? { read: "", send: "", connect: "" };
  if (t.read?.trim()) dirs.push({ name: "proxy_read_timeout", args: [t.read.trim()] });
  if (t.send?.trim()) dirs.push({ name: "proxy_send_timeout", args: [t.send.trim()] });
  if (t.connect?.trim()) dirs.push({ name: "proxy_connect_timeout", args: [t.connect.trim()] });

  if (state.bodySize?.trim()) dirs.push({ name: "client_max_body_size", args: [state.bodySize.trim()] });

  for (const h of state.addHeaders ?? []) {
    const name = h.name.trim();
    if (!name && !h.value.trim()) continue;
    dirs.push({ name: "add_header", args: [name, h.value] });
  }

  for (const r of state.rewrites ?? []) {
    const from = r.from.trim();
    const to = r.to.trim();
    if (!from && !to) continue;
    const args = [from, to];
    if (r.flag?.trim()) args.push(r.flag.trim());
    dirs.push({ name: "rewrite", args });
  }

  if (state.httpVersion?.trim()) dirs.push({ name: "proxy_http_version", args: [state.httpVersion.trim()] });
  if (state.buffering?.trim()) dirs.push({ name: "proxy_buffering", args: [state.buffering.trim()] });

  dirs.push(...parseFreeformDirectives(state.advanced ?? "", context));

  for (const d of dirs) assertValidDirective(d.name, d.args, context);
  return dirs;
}

// ---------------------------------------------------------------------------
// Directive[] -> state  (used by the edit page to prefill the form)
// ---------------------------------------------------------------------------

/**
 * Tries to place one directive into a structured field of `s`. Returns true on
 * success, false if the directive has no matching structured slot (or a
 * single-value slot is already taken).
 */
function tryHoistDirective(s: DirectiveEditorState, d: Directive): boolean {
  switch (d.name) {
    case "proxy_set_header":
      if (d.args.length === 2) {
        s.headers.push({ name: d.args[0]!, value: d.args[1]! });
        return true;
      }
      return false;
    case "add_header":
      if (d.args.length === 2) {
        s.addHeaders.push({ name: d.args[0]!, value: d.args[1]! });
        return true;
      }
      return false;
    case "proxy_read_timeout":
      if (d.args.length === 1 && !s.timeouts.read) {
        s.timeouts.read = d.args[0]!;
        return true;
      }
      return false;
    case "proxy_send_timeout":
      if (d.args.length === 1 && !s.timeouts.send) {
        s.timeouts.send = d.args[0]!;
        return true;
      }
      return false;
    case "proxy_connect_timeout":
      if (d.args.length === 1 && !s.timeouts.connect) {
        s.timeouts.connect = d.args[0]!;
        return true;
      }
      return false;
    case "client_max_body_size":
      if (d.args.length === 1 && !s.bodySize) {
        s.bodySize = d.args[0]!;
        return true;
      }
      return false;
    case "rewrite":
      if (d.args.length === 2 || (d.args.length === 3 && REWRITE_FLAGS.has(d.args[2]!))) {
        s.rewrites.push({ from: d.args[0]!, to: d.args[1]!, flag: d.args[2] ?? "" });
        return true;
      }
      return false;
    case "proxy_http_version":
      if (d.args.length === 1 && !s.httpVersion) {
        s.httpVersion = d.args[0]!;
        return true;
      }
      return false;
    case "proxy_buffering":
      if (d.args.length === 1 && !s.buffering) {
        s.buffering = d.args[0]!;
        return true;
      }
      return false;
    default:
      return false;
  }
}

/**
 * Regroups a stored `Directive[]` back into editor state (round-trip for the
 * edit page). Leading directives that map cleanly to structured fields are
 * hoisted; **from the first directive that cannot be hoisted onward, every
 * remaining directive is kept verbatim in the advanced block, in order.** This
 * preserves the original relative order across the structured/advanced boundary
 * so an edit+save never silently reorders order-sensitive directives (e.g. a
 * `return` in advanced followed by a `rewrite`). Nothing is ever lost.
 */
export function directivesToState(dirs: Directive[]): DirectiveEditorState {
  const s = emptyDirectiveState();
  const advanced: string[] = [];
  let fellBack = false;
  for (const d of dirs) {
    if (!fellBack && tryHoistDirective(s, d)) continue;
    fellBack = true;
    advanced.push(directiveToLine(d));
  }
  s.advanced = advanced.join("\n");
  return s;
}
