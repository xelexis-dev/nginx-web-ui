import { renderToStaticMarkup as render } from "react-dom/server";
import { expect, it } from "vitest";
import {
  Banner,
  Icon,
  CodePane,
  EmptyState,
  Facts,
  Modal,
  PageHeader,
  SettingRow,
  SettingRows,
  StatusBadge,
  Surface,
  Table,
  Td,
  Th,
  Tr,
} from "@repo/ui";

it("renders setting rows with separate status, wide detail, action and full-width message", () => {
  const html = render(
    <SettingRows>
      <SettingRow
        label="비밀번호"
        status={<StatusBadge tone="success">설정됨</StatusBadge>}
        detail="로그인에 사용합니다"
        action={<button>변경</button>}
        message={<p role="alert">실패</p>}
      />
      <SettingRow label="API 접근" wideDetail detail="자동화 도구" />
    </SettingRows>,
  );
  expect(html).toContain("<ul");
  expect(html.match(/<li\b/g)).toHaveLength(2);
  expect(html).toContain('data-wide=""');
  expect(html).toContain('role="alert"');
  for (const text of [
    "비밀번호",
    "설정됨",
    "로그인에 사용합니다",
    "변경",
    "자동화 도구",
  ])
    expect(html).toContain(text);
});

it("opts into facts notes only when a note is present, retaining every value", () => {
  const items = [
    { term: "대상", value: "서울" },
    { term: "백업", value: "가능", note: "등록 시 확인" },
  ];
  const html = render(<Facts items={items} />);
  expect(html).toContain('data-notes=""');
  expect(html.match(/<dd\b/g)).toHaveLength(4);
  expect(html).toContain("등록 시 확인");
  expect(render(<Facts items={[items[0]!]} />)).not.toContain("data-notes");
});

it("keeps status and tags outside the page and section headings", () => {
  const html = render(
    <>
      <PageHeader
        title="서버"
        status={<span>사용 중</span>}
        description="서울"
      />
      <Surface title="패스키" tag={<span>3/10</span>}>
        내용
      </Surface>
    </>,
  );
  expect(html).toMatch(/<h1[^>]*>서버<\/h1><span>사용 중<\/span>/);
  expect(html).toMatch(/<h2[^>]*>패스키<\/h2><span>3\/10<\/span>/);
});

it("keeps table defaults and emits opt-in layout, density and narrow attributes", () => {
  expect(render(<Table label="기본">내용</Table>)).toContain(
    'data-density="dense" data-layout="auto" data-narrow="summary"',
  );
  const html = render(
    <Table label="목록" layout="fixed" narrow="stacked" density="comfortable">
      <thead>
        <tr>
          <Th>이름</Th>
        </tr>
      </thead>
      <tbody>
        <Tr>
          <Td colSpan={1}>
            <EmptyState description="등록한 항목이 없습니다" />
          </Td>
        </Tr>
      </tbody>
    </Table>,
  );
  expect(html).toContain('data-layout="fixed" data-narrow="stacked"');
  expect(html).toContain('scope="col"');
  expect(html).toContain('colSpan="1"');
  expect(html).not.toMatch(/<p[^>]*><\/p>/);
});

it("renders a locked review dialog with disabled close and associated description", () => {
  const html = render(
    <Modal
      open
      size="review"
      dismissible={false}
      title="설정 적용"
      description="변경 3건"
      onClose={() => {}}
    >
      검토
    </Modal>,
  );
  expect(html).toContain('data-size="review"');
  expect(html).toContain('data-toast-reserve=""');
  expect(html).toMatch(/<button[^>]*disabled=""[^>]*aria-label="닫기"/);
});

it("renders secret text escaped without line numbers and retains copy and error states", () => {
  const html = render(
    <CodePane title="토큰" variant="secret" code="<secret>" />,
  );
  expect(html).toContain('data-variant="secret"');
  expect(html).toContain("&lt;secret&gt;");
  expect(html).toContain("복사");
  expect(html).not.toContain('aria-hidden="true">1</span>');
  const error = render(
    <CodePane title="토큰" variant="secret" code="hidden" error="실패" />,
  );
  expect(error).toContain('role="alert"');
  expect(error).not.toContain("hidden");
});

it.each([
  ["success", "circleCheck"],
  ["warning", "alert"],
  ["error", "circleAlert"],
  ["info", "info"],
] as const)("uses the shared %s banner icon", (tone, name) => {
  expect(render(<Banner tone={tone} title="결과" />)).toContain(
    render(<Icon name={name} size={16} />),
  );
});
