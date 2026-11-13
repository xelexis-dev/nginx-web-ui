import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { Button, IconButton, TextLink, Checkbox, FileInput, Icon, TextArea, buttonClassName, linkClassName } from "@repo/ui";

describe("action semantics", () => {
  it("keeps native form attributes and caller classes", () => {
    const html = renderToStaticMarkup(<Button type="submit" form="settings" name="action" value="save" disabled className="caller">저장 중…</Button>);
    for (const attr of ['type="submit"', 'form="settings"', 'name="action"', 'value="save"', 'disabled=""', 'caller', '저장 중…']) expect(html).toContain(attr);
    expect(renderToStaticMarkup(<Button>저장</Button>)).not.toContain('type=');
  });

  it("defaults row icons to ghost and utility icons to secondary", () => {
    for (const [size, variant] of [["sm", "ghost"], ["md", "secondary"]] as const) {
      const html = renderToStaticMarkup(<IconButton label="새로고침" size={size}>↻</IconButton>);
      expect(html).toContain('aria-label="새로고침"');
      expect(html).toContain('type="button"');
      for (const cls of buttonClassName({ size, variant }).split(" ")) expect(html).toContain(cls);
    }
  });

  it("keeps explicit icon variants, native disabled and expanded state", () => {
    const html = renderToStaticMarkup(<IconButton size="sm" variant="danger" label="삭제" disabled aria-expanded className="caller">×</IconButton>);
    expect(html).toContain('disabled=""');
    expect(html).toContain('aria-expanded="true"');
    expect(html).toContain('caller');
    for (const cls of buttonClassName({ size: "sm", variant: "danger" }).split(" ")) expect(html).toContain(cls);
  });
});

it("preserves native navigation attributes and custom classes for both link forms", () => {
  const html = renderToStaticMarkup(<TextLink href="/history#latest" target="_blank" rel="noreferrer" variant="back" className="caller">적용 이력</TextLink>);
  for (const value of ['href="/history#latest"', 'target="_blank"', 'rel="noreferrer"', 'caller', '적용 이력']) expect(html).toContain(value);
  for (const cls of linkClassName({ variant: "back" }).split(" ")) expect(html).toContain(cls);
});


it("marks only primary actions and keeps all four sizes on buttons and links", () => {
  for (const size of ["xs", "sm", "md", "lg"] as const) {
    const html = renderToStaticMarkup(<Button size={size}>저장</Button>);
    expect(html).toContain("nwui-btn-primary");
    for (const cls of buttonClassName({ size }).split(" ")) expect(html).toContain(cls);
  }
  for (const variant of ["secondary", "ghost", "danger", "dangerQuiet"] as const) {
    expect(renderToStaticMarkup(<Button variant={variant}>실행</Button>)).not.toContain("nwui-btn-primary");
  }
  const html = renderToStaticMarkup(<Button aria-busy disabled><Icon name="loaderCircle" />저장 중…</Button>);
  expect(html).toContain('aria-busy="true"');
  expect(html).toContain('disabled=""');
  expect(html).toContain('aria-hidden="true"');
});

it("keeps native checkbox/file attributes and raw textarea wrapping", () => {
  const html = renderToStaticMarkup(<Checkbox label="확인" name="confirmed" value="yes" defaultChecked required description="저장 전 확인" />);
  const input = html.match(/<input[^>]*>/)![0];
  for (const attr of ['type="checkbox"', 'name="confirmed"', 'value="yes"', 'checked=""', 'required=""', 'aria-describedby=']) expect(input).toContain(attr);
  expect(input).not.toContain("aria-hidden");
  expect(renderToStaticMarkup(<FileInput accept=".pem" disabled aria-label="파일" />)).toContain('type="file"');
  expect(renderToStaticMarkup(<TextArea mono name="config" defaultValue="server {}" />)).toContain('wrap="off"');
});
