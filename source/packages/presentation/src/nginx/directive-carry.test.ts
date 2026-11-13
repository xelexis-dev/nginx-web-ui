import { describe, expect, it } from "vitest";
import { carryDirectives, directiveStateUnchanged } from "./directive-carry";
import { directivesToState, emptyDirectiveState, seededLocationDirectiveState } from "./directive-state";
import type { Directive } from "./types";

describe("directive carry", () => {
  const original: Directive[] = [
    { name: "add_header", args: ["X-First", "it's a value"] },
    { name: "proxy_read_timeout", args: ["300s"] },
    { name: "proxy_set_header", args: ["Host", "$host"] },
    { name: "proxy_set_header", args: ["X-Empty", ""] },
    { name: "return", args: ["200", "ok"] },
  ];

  it("returns the untouched original array itself, retaining order and empty/quoted values", () => {
    const before = JSON.stringify(original);
    expect(carryDirectives(original, directivesToState(original))).toBe(original);
    expect(JSON.stringify(original)).toBe(before);
  });

  it("returns the edited editor state without parsing or dropping in-progress input", () => {
    const current = directivesToState(original);
    current.timeouts.read = "invalid while typing";
    expect(carryDirectives(original, current)).toBe(current);
  });

  it("distinguishes stored absence, an explicit empty array, and a new set", () => {
    expect(carryDirectives(null, emptyDirectiveState())).toBeNull();
    const explicit: Directive[] = [];
    expect(carryDirectives(explicit, emptyDirectiveState())).toBe(explicit);
    const newEmpty = emptyDirectiveState();
    expect(carryDirectives(undefined, newEmpty)).toBe(newEmpty);
    const seeded = seededLocationDirectiveState();
    expect(carryDirectives(undefined, seeded)).toBe(seeded);
    expect(carryDirectives(null, seeded)).toBe(seeded);
  });

  it("uses field values rather than object-property insertion order, and carries a reverted edit", () => {
    const initial = directivesToState(original);
    const reordered = { ...initial, timeouts: { connect: initial.timeouts.connect, send: initial.timeouts.send, read: initial.timeouts.read } };
    expect(directiveStateUnchanged(original, reordered)).toBe(true);
    initial.timeouts.read = "1s";
    expect(directiveStateUnchanged(original, initial)).toBe(false);
    initial.timeouts.read = "300s";
    expect(carryDirectives(original, initial)).toBe(original);
  });
});
