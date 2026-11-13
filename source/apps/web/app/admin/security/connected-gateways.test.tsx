import { describe, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import type { ComponentProps } from "react";
import type { GatewayView } from "@repo/contracts";
import { ConnectedGateways } from "./connected-gateways";

vi.mock("@/components/api-form", () => ({
  ApiForm: ({ children }: ComponentProps<"form">) => <form>{children}</form>,
}));

const row: GatewayView = {
  id: "gateway",
  name: "이전 게이트웨이",
  role: "viewer",
  createdAt: "2026-01-01T00:00:00Z",
  lastUsedAt: null,
  lastUsedIp: null,
  revokedAt: null,
};
const label = "이전 연결 · 발급자 확인 필요";
const notice =
  "현재 활성·승인된 관리자와 연결되지 않은 이전 연결입니다. 호환성을 위해 기존 접근을 유지합니다. 필요하지 않으면 연결을 끊으세요.";
const render = (extra: Record<string, unknown> = {}) =>
  renderToStaticMarkup(<ConnectedGateways rows={[{ ...row, ...extra }]} />);

describe("legacy gateway issuer reporting", () => {
  it("warns on an active orphan and keeps the explicit revoke action", () => {
    const html = render({ legacyOrphaned: true });
    expect(html).toContain(notice);
    expect(html).toMatch(/data-tone="warning"[^>]*role="status"/);
    expect(html).toMatch(
      new RegExp('data-tone="warning"[^>]*>(?:<span[^>]*></span>)?' + label),
    );
    expect(html).toContain("연결됨");
    expect(html).toContain('name="id" value="gateway"');
    expect(html).toContain("연결 끊기");
  });
  it("marks historical revoked orphans neutrally without an active warning or revoke action", () => {
    const html = render({
      legacyOrphaned: true,
      revokedAt: "2026-02-01T00:00:00Z",
    });
    expect(html).toMatch(
      new RegExp('data-tone="neutral"[^>]*>(?:<span[^>]*></span>)?' + label),
    );
    expect(html).toContain("해지됨");
    expect(html).not.toContain(notice);
    // The row offers no revoke action; the heading's info-tip trigger is the only button.
    expect(html.split("<tbody>")[1]).not.toContain("<button");
    expect(html.split("<tbody>")[1]).not.toContain("연결 끊기");
  });
  it.each([false, undefined, null, "true", 1, {}])(
    "does not infer orphan status from %j",
    (legacyOrphaned) => {
      const html = render({ legacyOrphaned });
      expect(html).not.toContain(label);
      expect(html).not.toContain(notice);
      expect(html).toContain("연결 끊기");
    },
  );
  it("keeps the empty state", () => {
    expect(renderToStaticMarkup(<ConnectedGateways rows={[]} />)).toContain(
      "연결된 게이트웨이가 없습니다.",
    );
  });
});
