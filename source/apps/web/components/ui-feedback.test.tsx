import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import {
  Banner,
  CodePane,
  Modal,
  Select,
  Combobox,
  StatusBadge,
  StatusDot,
  KpiRow,
  Facts,
} from "@repo/ui";
import { ConfirmDialog } from "../../../packages/ui/src/confirm-dialog";

it("keeps error alerts, explicit roles, raw output escaping and action labels", () => {
  const html = renderToStaticMarkup(
    <Banner
      tone="error"
      title="검사 실패"
      output={"<nginx>\nline 2"}
      action={<button>다시 검사</button>}
      onDismiss={() => {}}
    >
      설정을 확인하세요.
    </Banner>,
  );
  expect(html).toContain('role="alert"');
  expect(html).toContain("&lt;nginx&gt;\nline 2");
  expect(html).toContain("다시 검사");
  expect(html).toContain('aria-label="알림 닫기"');
  expect(
    renderToStaticMarkup(<Banner tone="error" title="실패" role="alert" />),
  ).toContain('role="alert"');
});
it("allows hiding or overriding the decorative tone icon", () => {
  expect(
    renderToStaticMarkup(<Banner tone="info" title="안내" icon={false} />),
  ).not.toContain("<svg");
  expect(
    renderToStaticMarkup(<Banner tone="info" title="안내" icon="check" />),
  ).toContain('aria-hidden="true"');
});
it("code previews retain escaped text, copy opt-out and error semantics", () => {
  const html = renderToStaticMarkup(
    <CodePane title="설정" code={"<upstream>\nlisten 80;"} copyLabel={false} />,
  );
  expect(html).toContain("&lt;upstream&gt;\n");
  expect(html).toContain("listen 80;");
  expect(html.match(/aria-hidden="true">[12]<\/span>/g)).toHaveLength(2);
  expect(html).not.toContain("<button");
  const error = renderToStaticMarkup(
    <CodePane title="설정" code="hidden" error="미리보기 실패" />,
  );
  expect(error).toContain('role="alert"');
  expect(error).not.toContain("hidden");
});
it("Modal labels its native dialog and links an optional description", () => {
  const html = renderToStaticMarkup(
    <Modal open title="확인" description="내용 확인" onClose={() => {}}>
      <button>본문</button>
    </Modal>,
  );
  expect(html).toContain("<dialog");
  const id = html.match(/aria-describedby="([^"]+)"/)?.[1];
  expect(id).toBeTruthy();
  expect(html).toContain('id="' + id + '"');
  expect(html).toContain('aria-label="닫기"');
  expect(html.match(/<svg\b/g)).toHaveLength(1);
  expect(html).not.toContain('data-tone="warning"');
  expect(html).not.toContain('data-tone="danger"');
  expect(
    renderToStaticMarkup(
      <Modal open={false} title="안내" onClose={() => {}}>
        본문
      </Modal>,
    ),
  ).not.toContain("aria-describedby");
});
it.each([false, true])(
  "ConfirmDialog preserves the target and controlled busy state (%s)",
  (pending) => {
    const html = renderToStaticMarkup(
      <ConfirmDialog
        open
        title="서버 삭제"
        description="되돌릴 수 없습니다."
        target="<서울 운영>"
        confirmLabel="삭제"
        pending={pending}
        onClose={() => {}}
        onConfirm={() => {}}
      />,
    );
    expect(html).toContain("&lt;서울 운영&gt;");
    expect(html).toContain('data-tone="danger"');
    expect(html).toContain("취소");
    expect(html).toContain(pending ? "처리 중..." : "삭제");
    expect(html.includes('aria-busy="true"')).toBe(pending);
    expect(/<button[^>]*disabled/.test(html)).toBe(pending);
  },
);

it("keeps explicit polite errors and non-error status roles", () => {
  expect(
    renderToStaticMarkup(<Banner tone="error" title="실패" role="status" />),
  ).toContain('role="status"');
  expect(renderToStaticMarkup(<Banner tone="info" title="안내" />)).toContain(
    'role="status"',
  );
});
it("a one-line info alert has no title paragraph", () => {
  const html = renderToStaticMarkup(<Banner tone="info" title="안내" />);
  expect(html).toContain("안내");
  expect(html).not.toMatch(/<p(?:\s|>)/);
});
it("preserves native form values for both dropdowns", () => {
  const html = renderToStaticMarkup(
    <form>
      <Select
        name="protocol"
        defaultValue="udp"
        options={[{ value: "udp", label: "UDP" }]}
      />
      <Combobox
        name="header"
        value="X-Custom"
        options={[]}
        onChange={() => {}}
      />
    </form>,
  );
  expect(html).toContain('type="hidden" name="protocol" value="udp"');
  expect(html).toMatch(/<input[^>]*name="header"[^>]*value="X-Custom"/);
});
it("keeps status meaning in labels, decorative shapes and tag attributes", () => {
  const status = renderToStaticMarkup(
    <StatusBadge tone="warning">주의</StatusBadge>,
  );
  expect(status).toContain("주의");
  expect(status).toContain('aria-hidden="true"');
  expect(status).not.toContain("data-tag");
  expect(
    renderToStaticMarkup(
      <StatusBadge tone="info" dot={false}>
        v2
      </StatusBadge>,
    ),
  ).toContain('data-tag=""');
  expect(renderToStaticMarkup(<StatusDot tone="error" />)).toContain(
    'data-size="8"',
  );
});
it("retains all data and dl semantics for readouts and facts", () => {
  const html = renderToStaticMarkup(
    <>
      <KpiRow
        label="연결"
        items={[{ label: "연결 수", value: "1,284", note: "현재" }]}
      />
      <Facts items={[{ term: "대상", value: "<server>", wide: true }]} />
    </>,
  );
  expect(html.match(/<dl/g)).toHaveLength(2);
  expect(html).toContain('aria-label="연결"');
  for (const text of ["연결 수", "1,284", "현재", "대상", "&lt;server&gt;"])
    expect(html).toContain(text);
  expect(html).toContain('data-wide=""');
});

it.each(["warning", "danger"] as const)(
  "Modal opts into a decorative %s icon",
  (icon) => {
    const html = renderToStaticMarkup(
      <Modal open title="확인" icon={icon} onClose={() => {}}>
        내용
      </Modal>,
    );
    expect(html).toContain(`data-tone="${icon}"`);
    expect(html.match(/<svg\b/g)).toHaveLength(2);
    expect(html).toContain('aria-hidden="true"');
  },
);
