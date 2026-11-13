import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { StreamList } from "./stream-list";

describe.each(["/stream", "/nodes/node-1/stream"])("StreamList %s", (basePath) => {
  it("keeps creation and detail links without a second apply control", () => {
    const html = renderToStaticMarkup(<StreamList basePath={basePath} data={{
      canWrite: true,
      items: [{ id: "st-1", listen: 5432, protocol: "tcp", proxyPass: "database:5432", enabled: true }],
    }} />);
    expect(html).toContain(`href="${basePath}/new"`);
    expect(html).toContain(`href="${basePath}/st-1"`);
    expect(html).toContain("database:5432");
    expect(html).not.toContain("설정 적용");
    expect(html).not.toContain("<form");
  });
  it("keeps the empty state read-only for viewers", () => {
    const html = renderToStaticMarkup(<StreamList basePath={basePath} data={{ canWrite: false, items: [] }} />);
    expect(html).toContain("등록된 stream이 없습니다");
    expect(html).not.toContain(`${basePath}/new`);
    expect(html).not.toContain("설정 적용");
  });
});
