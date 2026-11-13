import { describe, expect, it } from "vitest";
import { describeMismatch } from "./version";

describe("gateway-first mismatch advice", () => {
  it.each([
    ["2.0.0", "2.1.0", "게이트웨이를 올리세요"],
    ["2.1.0", "1.19.11", "노드를 올리세요"],
    ["1.19.11", "2.1.0", "게이트웨이를 올리세요"],
    ["1.18.9", "2.0.0", "게이트웨이를 올리세요"],
    ["3.0.0", "2.9.0", "노드를 올리세요"],
  ])("%s -> %s names the older side", (gateway, node, advice) => {
    expect(describeMismatch(gateway, node)).toContain(advice);
    expect(describeMismatch(gateway, node)).toContain(`게이트웨이 ${gateway} / 노드 ${node}`);
  });
});
