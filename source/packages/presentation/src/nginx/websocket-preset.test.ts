import { describe, expect, it } from "vitest";
import { emptyDirectiveState, seededLocationDirectiveState, stateToDirectives, type DirectiveEditorState } from "./directive-state";
import { applyWebSocketPreset } from "./websocket-preset";

const headerDirectives = (state: DirectiveEditorState, name: string) => stateToDirectives(state, "location")
  .filter(d => d.name === "proxy_set_header" && d.args[0]?.toLowerCase() === name.toLowerCase());
function frozenState(state: DirectiveEditorState): DirectiveEditorState {
  for (const header of [...state.headers, ...state.addHeaders]) Object.freeze(header);
  for (const rewrite of state.rewrites) Object.freeze(rewrite);
  Object.freeze(state.headers); Object.freeze(state.addHeaders); Object.freeze(state.rewrites); Object.freeze(state.timeouts);
  return Object.freeze(state);
}

describe("WebSocket directive preset", () => {
  it("fills ordinary HTTP/1.1 and conditional upgrade directives without an extra persisted flag", () => {
    const result = applyWebSocketPreset(emptyDirectiveState());
    expect(result.status).toBe("applied");
    expect(result.conflicts).toEqual([]);
    expect(stateToDirectives(result.state, "location")).toEqual([
      { name: "proxy_set_header", args: ["Upgrade", "$http_upgrade"] },
      { name: "proxy_set_header", args: ["Connection", "$connection_upgrade"] },
      { name: "proxy_http_version", args: ["1.1"] },
    ]);
  });

  it("preserves default Host and forwarding headers, timeouts, buffering and other settings without mutating input", () => {
    const original = seededLocationDirectiveState();
    original.headers.push({ name: "X-Custom", value: " token " });
    original.timeouts = { read: "75s", send: "80s", connect: "5s" };
    original.buffering = "on"; original.bodySize = "20m";
    original.addHeaders = [{ name: "X-Response", value: "yes" }];
    original.rewrites = [{ from: "^/a/(.*)", to: "/b/$1", flag: "break" }];
    original.advanced = "# keep this\nproxy_redirect off;\n";
    const before = structuredClone(original);
    const result = applyWebSocketPreset(frozenState(original));
    expect(result.status).toBe("applied");
    expect(original).toEqual(before);
    expect(result.state.headers.slice(0, 5)).toEqual(before.headers);
    expect(result.state.timeouts).toEqual(before.timeouts);
    expect(result.state.buffering).toBe("on"); expect(result.state.bodySize).toBe("20m");
    expect(result.state.addHeaders).toEqual(before.addHeaders); expect(result.state.rewrites).toEqual(before.rewrites);
    expect(result.state.advanced).toBe(before.advanced);
    expect(headerDirectives(result.state, "Host")).toEqual([{ name: "proxy_set_header", args: ["Host", "$host"] }]);
  });

  it("is idempotent and retains an already-correct case-insensitive header name", () => {
    const state = emptyDirectiveState();
    state.httpVersion = "1.1";
    state.headers = [{ name: "uPgRaDe", value: "$http_upgrade" }, { name: " CONNECTION ", value: "$connection_upgrade" }];
    const first = applyWebSocketPreset(state);
    const second = applyWebSocketPreset(first.state);
    expect(first.status).toBe("applied"); expect(second.status).toBe("applied");
    expect(first.state).toEqual(state); expect(second.state).toEqual(first.state);
    expect(headerDirectives(second.state, "Upgrade")).toHaveLength(1);
    expect(headerDirectives(second.state, "Connection")).toHaveLength(1);
  });

  it.each([
    ["httpVersion", "1.0"], ["Upgrade", ""], ["Connection", "upgrade"], ["Connection", "$http_connection"],
  ])("requires explicit replacement for conflicting %s=%s", (field, value) => {
    const state = emptyDirectiveState();
    if (field === "httpVersion") state.httpVersion = value;
    else state.headers = [{ name: field, value }];
    const result = applyWebSocketPreset(frozenState(state));
    expect(result.status).toBe("conflict"); expect(result.state).toBe(state);
    expect(result.conflicts).toMatchObject([{ field, existingValues: [value] }]);
    const replaced = applyWebSocketPreset(state, "replace");
    expect(replaced.status).toBe("applied"); expect(replaced.state.httpVersion).toBe("1.1");
    expect(headerDirectives(replaced.state, "Upgrade")).toEqual([{ name: "proxy_set_header", args: ["Upgrade", "$http_upgrade"] }]);
    expect(headerDirectives(replaced.state, "Connection")).toEqual([{ name: "proxy_set_header", args: ["Connection", "$connection_upgrade"] }]);
    expect(applyWebSocketPreset(replaced.state).state).toEqual(replaced.state);
  });

  it("does not silently deduplicate even matching duplicate headers", () => {
    const state = emptyDirectiveState();
    state.headers = [{ name: "Upgrade", value: "$http_upgrade" }, { name: "upgrade", value: "$http_upgrade" }];
    expect(applyWebSocketPreset(state).status).toBe("conflict");
    const result = applyWebSocketPreset(state, "replace");
    expect(headerDirectives(result.state, "Upgrade")).toHaveLength(1);
  });

  it("keeps correct advanced entries where they are rather than adding ineffective duplicate slots", () => {
    const state = seededLocationDirectiveState();
    state.advanced = '# comment\r\nproxy_set_header "Upgrade" "$http_upgrade";\r\nproxy_set_header Connection $connection_upgrade;\r\nproxy_http_version 1.1;\r\nproxy_redirect off;';
    const result = applyWebSocketPreset(state);
    expect(result.status).toBe("applied"); expect(result.state).toEqual(state);
    expect(headerDirectives(result.state, "Upgrade")).toHaveLength(1);
    expect(headerDirectives(result.state, "Connection")).toHaveLength(1);
    expect(stateToDirectives(result.state, "location").filter(d => d.name === "proxy_http_version")).toHaveLength(1);
  });

  it("replaces only conflicting advanced/structured slots and preserves unrelated advanced bytes and empty Host", () => {
    const state = emptyDirectiveState();
    state.headers = [{ name: "Host", value: "" }, { name: "Upgrade", value: "$http_upgrade" }];
    state.httpVersion = "1.0";
    state.advanced = '# preserve\r\nproxy_redirect off;\r\nproxy_set_header upgrade "";\r\nproxy_set_header Connection upgrade;\r\nproxy_http_version 1.0;\r\nadd_header Connection close;\r\n';
    const blocked = applyWebSocketPreset(state);
    expect(blocked.status).toBe("conflict"); expect(blocked.state).toBe(state);
    expect(blocked.conflicts.map(c => c.field)).toEqual(["httpVersion", "Upgrade", "Connection"]);
    const result = applyWebSocketPreset(state, "replace");
    expect(result.state.advanced).toBe('# preserve\r\nproxy_redirect off;\r\nadd_header Connection close;\r\n');
    expect(headerDirectives(result.state, "Host")).toEqual([{ name: "proxy_set_header", args: ["Host", ""] }]);
    expect(headerDirectives(result.state, "Upgrade")).toHaveLength(1);
    expect(headerDirectives(result.state, "Connection")).toHaveLength(1);
    expect(stateToDirectives(result.state, "location").filter(d => d.name === "proxy_http_version")).toEqual([{ name: "proxy_http_version", args: ["1.1"] }]);
    expect(result.state.advanced).toContain("add_header Connection close;");
    expect(applyWebSocketPreset(result.state).state).toEqual(result.state);
  });

  it("leaves invalid advanced input intact and exposes an inline validation error even when replacement was chosen", () => {
    const state = emptyDirectiveState(); state.advanced = 'proxy_set_header Upgrade "unterminated';
    const result = applyWebSocketPreset(state, "replace");
    expect(result.status).toBe("invalid"); expect(result.state).toBe(state); expect(result.conflicts).toEqual([]);
    if (result.status === "invalid") expect(result.error).toContain("Unterminated");
  });
});

it.each([
  "proxy_http_version 1.1 extra;",
  "proxy_set_header Upgrade $http_upgrade extra;",
])("preserves invalid target directive arity instead of replacing it: %s", (advanced) => {
  const state = emptyDirectiveState(); state.advanced = advanced;
  const result = applyWebSocketPreset(frozenState(state), "replace");
  expect(result.status).toBe("invalid"); expect(result.state).toBe(state); expect(result.conflicts).toEqual([]);
  if (result.status === "invalid") expect(result.error).toContain("argument");
});
