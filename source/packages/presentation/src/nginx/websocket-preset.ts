import type { DirectiveEditorState } from "./directive-state";
import { parseFreeformDirectives } from "./directive-validation";
import type { Directive } from "./types";

export type WebSocketPresetField = "httpVersion" | "Upgrade" | "Connection";
export type WebSocketPresetResolution = "preserve" | "replace";

export interface WebSocketPresetConflict {
  field: WebSocketPresetField;
  existingValues: string[];
  expectedValue: string;
}

export type WebSocketPresetResult =
  | { status: "applied"; state: DirectiveEditorState; conflicts: WebSocketPresetConflict[] }
  | { status: "conflict"; state: DirectiveEditorState; conflicts: WebSocketPresetConflict[] }
  | { status: "invalid"; state: DirectiveEditorState; conflicts: []; error: string };

const SLOTS = [
  { field: "httpVersion", value: "1.1" },
  { field: "Upgrade", value: "$http_upgrade" },
  { field: "Connection", value: "$connection_upgrade" },
] as const;

function headerField(name: string): WebSocketPresetField | undefined {
  switch (name.trim().toLowerCase()) {
    case "upgrade": return "Upgrade";
    case "connection": return "Connection";
    default: return undefined;
  }
}

function directiveField(directive: Directive | undefined): WebSocketPresetField | undefined {
  if (directive?.name === "proxy_http_version") return "httpVersion";
  if (directive?.name === "proxy_set_header") return headerField(directive.args[0] ?? "");
  return undefined;
}

/**
 * Fill existing directive fields only; there is no persisted WebSocket flag.
 * Any duplicate or conflicting target setting blocks all changes until the
 * caller explicitly chooses replacement. Advanced lines participate too, so
 * the preset never hides an effective setting behind a structured duplicate.
 */
export function applyWebSocketPreset(
  state: DirectiveEditorState,
  resolution: WebSocketPresetResolution = "preserve",
): WebSocketPresetResult {
  const observed: Record<WebSocketPresetField, { value: string; compatible: boolean }[]> = {
    httpVersion: [], Upgrade: [], Connection: [],
  };
  const record = (field: WebSocketPresetField, value: string) => {
    const slot = SLOTS.find(slot => slot.field === field)!;
    observed[field].push({ value, compatible: value === slot.value });
  };
  if (state.httpVersion.trim()) record("httpVersion", state.httpVersion.trim());
  for (const header of state.headers) {
    const field = headerField(header.name);
    if (field) record(field, header.value);
  }

  // Keep chunks verbatim, including comments, whitespace and CRLF. The editor
  // contract is one directive per line, parsed by the existing validator.
  const chunks = state.advanced.match(/[^\n]*\n|[^\n]+$/g) ?? [];
  let advanced: { raw: string; field: WebSocketPresetField | undefined }[];
  try {
    advanced = chunks.map(raw => {
      const directive = parseFreeformDirectives(raw, "location")[0];
      const field = directiveField(directive);
      if (field && directive) {
        const expectedArguments = directive.name === "proxy_http_version" ? 1 : 2;
        if (directive.args.length !== expectedArguments) {
          throw new Error(`${directive.name} needs exactly ${expectedArguments} argument(s).`);
        }
        record(field, directive.args[directive.name === "proxy_http_version" ? 0 : 1]!);
      }
      return { raw, field };
    });
  } catch (error) {
    return { status: "invalid", state, conflicts: [], error: error instanceof Error ? error.message : String(error) };
  }

  const conflicts: WebSocketPresetConflict[] = SLOTS.flatMap(slot => {
    const existing = observed[slot.field];
    return existing.length > 1 || existing.some(value => !value.compatible)
      ? [{ field: slot.field, existingValues: existing.map(value => value.value), expectedValue: slot.value }]
      : [];
  });
  if (conflicts.length && resolution !== "replace") return { status: "conflict", state, conflicts };

  const replace = new Set(conflicts.map(conflict => conflict.field));
  const fill = new Set<WebSocketPresetField>(SLOTS.filter(slot => !observed[slot.field].length || replace.has(slot.field)).map(slot => slot.field));
  const next: DirectiveEditorState = {
    ...state,
    headers: state.headers.filter(header => {
      const field = headerField(header.name);
      return !field || !fill.has(field);
    }).map(header => ({ ...header })),
    timeouts: { ...state.timeouts },
    addHeaders: state.addHeaders.map(header => ({ ...header })),
    rewrites: state.rewrites.map(rewrite => ({ ...rewrite })),
    advanced: advanced.filter(line => !line.field || !replace.has(line.field)).map(line => line.raw).join(""),
  };
  if (fill.has("httpVersion")) next.httpVersion = "1.1";
  for (const slot of SLOTS) {
    if (slot.field !== "httpVersion" && fill.has(slot.field)) next.headers.push({ name: slot.field, value: slot.value });
  }
  return { status: "applied", state: next, conflicts };
}
