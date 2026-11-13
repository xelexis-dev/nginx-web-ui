import { expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { RuntimeDnsOption, RuntimeDnsNotice } from "./runtime-dns";
import { runtimeDnsModeLabel } from "./runtime-dns-state";

it("allows new dynamic targets only after the target instance reports a usable service", () => {
  const available = renderToStaticMarkup(<RuntimeDnsOption checked={false} onChange={() => {}} status={{ available: true, reason: null }} />);
  expect(available).toContain("실행 중에 이름 확인");
  expect(available).toContain('aria-checked="false"'); expect(available).not.toContain('disabled=""');
  const unavailable = renderToStaticMarkup(<RuntimeDnsOption checked={false} onChange={() => {}} status={{ available: false, reason: "5354 포트가 사용 중입니다." }} />);
  expect(unavailable).toContain('disabled=""'); expect(unavailable).toContain("5354 포트가 사용 중입니다.");
  const unknown = renderToStaticMarkup(<RuntimeDnsOption checked={false} onChange={() => {}} />);
  expect(unknown).toContain('disabled=""'); expect(unknown).toContain("상태를 확인할 수 없습니다");
});

it("preserves an already-enabled choice during an outage and still allows turning it off", () => {
  const html = renderToStaticMarkup(<RuntimeDnsOption checked onChange={() => {}} status={{ available: false, reason: "이름 확인 서비스가 준비되지 않았습니다." }} error="대상 이름을 확인하세요." />);
  expect(html).toContain('aria-checked="true"'); expect(html).not.toContain('disabled=""');
  expect(html).toContain("이름 확인 서비스가 준비되지 않았습니다."); expect(html).toContain("대상 이름을 확인하세요.");
});

it("shows selected mode independently of availability and never equates missing status with availability", () => {
  expect(runtimeDnsModeLabel(true)).toBe("실행 중에 이름 확인");
  expect(runtimeDnsModeLabel(false)).toBe("정적 (설정 적용 시)");
  const html = renderToStaticMarkup(<RuntimeDnsNotice status={{ available: false, reason: "포트 충돌" }} />);
  expect(html).toContain("포트 충돌");
  expect(renderToStaticMarkup(<RuntimeDnsNotice active />)).toContain("상태를 확인할 수 없습니다");
  expect(renderToStaticMarkup(<RuntimeDnsNotice status={{ available: true, reason: null }} />)).toBe("");
});
