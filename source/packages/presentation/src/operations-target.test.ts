import { expect, it } from "vitest";
import { targetLabel } from "./operations-target";
it("uses the typed target rather than interpreting a node name", () => {
  expect(targetLabel({ kind: "local" })).toBe("로컬 서버");
  for (const name of ["local", "admin", "긴 이름의 원격 노드"])
    expect(targetLabel({ kind: "node", id: "node-1", name })).toBe(name);
});
