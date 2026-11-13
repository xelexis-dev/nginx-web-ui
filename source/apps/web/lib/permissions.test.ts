import { expect, it } from "vitest";
import type { Action, Role } from "@repo/contracts";
import { can } from "./permissions";

const actions: Action[] = ["config:read", "config:write", "nginx:reload", "user:manage"];

it.each([
  ["viewer", [true, false, false, false]],
  ["editor", [true, true, true, false]],
  ["admin", [true, true, true, true]],
] satisfies Array<[Role, boolean[]]>)("%s sees only the permitted controls", (role, expected) => {
  expect(actions.map(action => can(role, action))).toEqual(expected);
});

it("unknown roles and purpose scopes never display privileged controls", () => {
  expect(can("unknown" as Role, "config:write")).toBe(false);
  expect(can("admin", "node:pair" as Action)).toBe(false);
});
