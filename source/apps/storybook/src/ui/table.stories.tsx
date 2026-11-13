import { EmptyState, Icon, StatusBadge, Switch, buttonClassName } from "@repo/ui";
import { Table, Td, Th, Tr } from "../../../../packages/ui/src/table";
import { expect, userEvent, within } from "storybook/test";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { useState } from "react";

// Storybook-only sample rows.
const rows = [
  { id: "a", name: "app.example.test", port: "80·443", target: "app_pool", tls: "HTTPS · 61일", rpm: "4,820", err: "0.2%", enabled: true, tone: "success" as const, status: "정상" },
  { id: "b", name: "api.example.test", port: "80·443", target: "경로 3개로 나눔", tls: "HTTPS · 61일", rpm: "12,640", err: "2.8%", hot: true, enabled: true, tone: "warning" as const, status: "주의" },
  { id: "c", name: "files.example.test", port: "80", target: "http://files.internal.example.test:9100", tls: "인증서 삭제됨", rpm: "138", err: "0.6%", enabled: true, tone: "error" as const, status: "확인 필요" },
  { id: "d", name: "legacy.example.test", port: "80", target: "http://legacy.internal.example.test:8000", tls: "HTTP", rpm: "—", err: "—", enabled: false, tone: "neutral" as const, status: "비활성", pending: true },
];

function ServerTable() {
  const [enabled, setEnabled] = useState<Record<string, boolean>>(Object.fromEntries(rows.map((r) => [r.id, r.enabled])));
  return (
    <Table label="서버 목록">
      <thead>
        <tr>
          <Th>상태</Th>
          <Th>서버</Th>
          <Th align="end">포트</Th>
          <Th>대상</Th>
          <Th>TLS</Th>
          <Th align="end">요청</Th>
          <Th align="end">5xx</Th>
          <Th>활성</Th>
          <Th>
            <span style={{ position: "absolute", width: 1, height: 1, overflow: "hidden", clipPath: "inset(50%)" }}>작업</span>
          </Th>
        </tr>
      </thead>
      <tbody>
        {rows.map((r) => (
          <Tr key={r.id} pending={r.pending}>
            <Td cell="status">
              <StatusBadge tone={r.tone}>{r.status}</StatusBadge>
            </Td>
            <Td cell="primary">
              <a href={`#${r.id}`}>{r.name}</a>
              {r.pending ? " · 미적용" : null}
            </Td>
            <Td numeric>{r.port}</Td>
            <Td mono truncate title={r.target}>
              {r.target}
            </Td>
            <Td>{r.tls}</Td>
            <Td numeric>{r.rpm}</Td>
            <Td numeric tone={r.hot ? "error" : undefined}>
              {r.err}
            </Td>
            <Td cell="action">
              <Switch label={`${r.name} 활성`} checked={enabled[r.id]!} onCheckedChange={(next) => setEnabled((prev) => ({ ...prev, [r.id]: next }))} />
            </Td>
            <Td cell="hide-narrow">
              <a href={`#${r.id}-edit`} className={buttonClassName({ variant: "ghost", size: "sm" })}>
                수정
              </a>
            </Td>
          </Tr>
        ))}
      </tbody>
    </Table>
  );
}

const meta = {
  title: "UI/Table",
  component: Table,
  decorators: [(Story) => <main style={{ maxWidth: 1200, width: "100%", margin: "0 auto" }}><h1>데이터 표</h1><Story /></main>],
  parameters: {
    docs: {
      subtitle: "하나의 표가 좁은 화면에서는 요약 행으로 다시 놓인다",
      description: {
        component: [
          "`Table`·`Th`·`Td`·`Tr`은 표 하나의 DOM이다. 행의 링크와 폼이 한 번만 존재한다.",
          "",
          "- 넓은 화면: 셀 여백 6 × 12(`--sp-1-5` × `--cell-x`), 행은 최소 36px(`--size-row`)이고 내용이 길면 늘어난다. 아래 구분선이 있는 머리, 숫자는 오른쪽 정렬(`numeric`), 원문은 `mono`, 한 줄로 줄일 값은 `truncate`(전체 값은 `title`), 줄을 바꿀 긴 값(주소·토큰)은 `wrap`.",
          "- 768px 이하: 각 행이 요약 블록이 된다. 첫 줄은 `cell=\"status\"` · `cell=\"primary\"` · `cell=\"action\"`, 다음 줄은 나머지(`meta`, 기본값)가 점으로 이어진다. `hide-narrow`는 숨긴다. 머리는 화면 읽기 프로그램에만 남는다.",
          "- 좁은 화면에서는 열 머리가 보이지 않으므로, 날짜·개수처럼 혼자서는 뜻을 모르는 값은 `label`을 준다. 점 대신 그 이름이 값 앞에 붙는다(화면 읽기 프로그램은 열 머리를 그대로 읽는다).",
          "- 행이 없으면 모든 열을 잇는 칸 하나(`colSpan`)에 `EmptyState`를 둔다.",
          "- `Tr pending`: 아직 적용하지 않은 변경이 있는 행. 색만으로 알리지 않도록 글자(예: 「미적용」)도 함께 쓴다.",
          "- 표 영역은 이름 있는 스크롤 영역이라 키보드로도 가로 스크롤할 수 있다.",
        ].join("\n"),
      },
    },
  },
  args: { label: "서버 목록", children: null },
  argTypes: { children: { control: false } },
} satisfies Meta<typeof Table>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Servers: Story = { render: () => <ServerTable /> };

/** 390px 폭 안에서: 행이 요약 블록으로 바뀐다. */
export const Narrow: Story = {
  render: () => <ServerTable />,
  globals: { viewport: { value: "ui390", isRotated: false } },
  parameters: { viewport: { options: { ui390: { name: "Mobile 390", styles: { width: "390px", height: "844px" }, type: "mobile" } } } },
};

const narrow = {
  globals: { viewport: { value: "ui390", isRotated: false } },
  parameters: { viewport: { options: { ui390: { name: "Mobile 390", styles: { width: "390px", height: "844px" }, type: "mobile" } } } },
};

// An admin list: the dates and the count mean nothing without their column, so they carry a label.
const users = [
  { id: "u1", email: "ops@example.test", role: "admin", active: true, twoFactor: "사용", created: "2026. 7. 2.", lastLogin: "2026. 9. 27. 오전 10:02", tokens: 2 },
  { id: "u2", email: "release-automation.long-service-account@deploy.example.test", role: "editor", active: true, twoFactor: "미사용", created: "2026. 8. 11.", lastLogin: "기록 없음", tokens: 0 },
  { id: "u3", email: "viewer@example.test", role: "viewer", active: false, twoFactor: "사용", created: "2026. 9. 1.", lastLogin: "2026. 9. 20. 오후 3:41", tokens: 1 },
];

function UsersTable() {
  return (
    <Table label="사용자 목록">
      <thead>
        <tr>
          <Th>상태</Th>
          <Th>이메일</Th>
          <Th>역할</Th>
          <Th>2FA</Th>
          <Th>가입</Th>
          <Th>마지막 로그인</Th>
          <Th align="end">토큰</Th>
          <Th>
            <span style={{ position: "absolute", width: 1, height: 1, overflow: "hidden", clipPath: "inset(50%)" }}>작업</span>
          </Th>
        </tr>
      </thead>
      <tbody>
        {users.map((u) => (
          <Tr key={u.id}>
            <Td cell="status">
              <StatusBadge tone={u.active ? "success" : "neutral"}>{u.active ? "활성" : "비활성"}</StatusBadge>
            </Td>
            <Td cell="primary" wrap>
              <a href={`#${u.id}`}>{u.email}</a>
            </Td>
            <Td label="역할">{u.role}</Td>
            <Td label="2FA">{u.twoFactor}</Td>
            <Td label="가입">{u.created}</Td>
            <Td label="마지막 로그인">{u.lastLogin}</Td>
            <Td numeric label="토큰">
              {u.tokens}
            </Td>
            <Td cell="action">
              <a href={`#${u.id}-edit`} className={buttonClassName({ variant: "ghost", size: "sm" })}>
                수정
              </a>
            </Td>
          </Tr>
        ))}
      </tbody>
    </Table>
  );
}

/** 관리자 목록 같은 표: 좁은 화면에서 날짜·개수 앞에 `label`이 붙는다(넓은 화면에서는 열 머리가 있다). */
export const Labelled: Story = { name: "이름표가 붙은 칸", render: () => <UsersTable /> };
export const LabelledNarrow: Story = { name: "이름표가 붙은 칸 · 390", render: () => <UsersTable />, ...narrow };

// Long values with no spaces: webhook URLs and token prefixes.
const hooks = [
  { id: "w1", name: "배포 알림", url: "https://hooks.example.test/services/T00000000/B00000000/XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX", token: "nwui_tok_3f9a1c7e2b8d4f60a1c3e5b7d9f1a3c5e7b9d1f3" },
  { id: "w2", name: "인증서 만료 알림(운영팀 공용 채널로 보내는 아주 긴 이름의 웹훅)", url: "https://chat.example.test/api/v2/webhooks/incoming/ops-team/certificate-expiry-notifications?token=abcdef0123456789", token: "nwui_tok_0000000000000000000000000000000000000000" },
];

function LongValuesTable() {
  return (
    <Table label="웹훅 목록">
      <thead>
        <tr>
          <Th>이름</Th>
          <Th>주소</Th>
          <Th>토큰</Th>
        </tr>
      </thead>
      <tbody>
        {hooks.map((h) => (
          <Tr key={h.id}>
            <Td cell="primary" wrap>
              <a href={`#${h.id}`}>{h.name}</a>
            </Td>
            <Td mono wrap label="주소">
              {h.url}
            </Td>
            <Td mono wrap label="토큰">
              {h.token}
            </Td>
          </Tr>
        ))}
      </tbody>
    </Table>
  );
}

/** 공백 없는 긴 주소·토큰: `wrap`이면 줄을 바꿔 페이지를 넘치지 않는다. */
export const LongValues: Story = { name: "긴 주소·토큰", render: () => <LongValuesTable /> };
export const LongValuesNarrow: Story = { name: "긴 주소·토큰 · 390", render: () => <LongValuesTable />, ...narrow };

/** 행이 없을 때: 모든 열을 잇는 칸 하나에 `EmptyState`. */
export const Empty: Story = {
  name: "빈 표",
  render: () => (
    <Table label="API 토큰 목록">
      <thead>
        <tr>
          <Th>이름</Th>
          <Th>권한</Th>
          <Th>만료</Th>
          <Th>마지막 사용</Th>
        </tr>
      </thead>
      <tbody>
        <Tr>
          <Td colSpan={4}>
            <EmptyState title="발급한 토큰이 없습니다" description="자동화 도구가 이 서버를 다루려면 토큰을 발급하세요." />
          </Td>
        </Tr>
      </tbody>
    </Table>
  ),
};

export const PrimaryDetails: Story = {
  name: "아이콘과 보조 정보 · 미적용",
  render: () => <Table label="적용할 서버 목록"><thead><tr><Th>서버 이름과 연결 대상의 상세 정보</Th><Th>상태</Th><Th>요청 수</Th></tr></thead><tbody>
    <Tr pending current><Td cell="primary" icon={<Icon name="servers" />} secondary="저장된 변경 사항이 아직 실행 중인 서버에 적용되지 않았습니다."><a href="#server">운영팀에서 관리하는 아주 긴 이름의 리버스 프록시 서버</a></Td><Td cell="status"><StatusBadge tone="warning">미적용</StatusBadge></Td><Td numeric label="요청 수">1,284</Td></Tr>
    <Tr><Td cell="primary" icon={<Icon name="stream" />} secondary="tcp://upstream.example.test:9000"><a href="#stream">TCP 연결</a></Td><Td cell="status"><StatusBadge tone="success">정상</StatusBadge></Td><Td numeric label="요청 수">0</Td></Tr>
  </tbody></Table>,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByRole("table", { name: "적용할 서버 목록" })).toBeVisible();
    await expect(canvas.getByRole("link", { name: "운영팀에서 관리하는 아주 긴 이름의 리버스 프록시 서버" })).toHaveAttribute("href", "#server");
    await expect(canvasElement.querySelectorAll("td svg[aria-hidden=true]")).toHaveLength(2);
    await expect(canvasElement.querySelectorAll("tr[data-state=pending]")).toHaveLength(1);
    await expect(canvas.getByText("저장된 변경 사항이 아직 실행 중인 서버에 적용되지 않았습니다.")).toBeVisible();
  },
};

export const Hover: Story = {
  name: "행에 포인터 올림",
  render: () => <ServerTable />,
  play: async ({ canvasElement }) => {
    await userEvent.hover(within(canvasElement).getByRole("row", { name: /app\.example\.test/ }));
  },
};

/** Deliberately shuffled DOM: mobile order must come from role-based layout. */
export const ReflowOrder: Story = {
  render: () => (
    <Table label="화면 순서 검증">
      <thead>
        <tr>
          <Th>이름</Th>
          <Th>요청</Th>
          <Th>작업</Th>
          <Th>상태</Th>
          <Th>설명</Th>
        </tr>
      </thead>
      <tbody>
        <Tr current>
          <Td cell="primary">
            <a href="#one">app.example.test</a>
          </Td>
          <Td label="요청">1,284</Td>
          <Td cell="action">
            <a href="#edit-one">수정</a>
          </Td>
          <Td cell="status">정상</Td>
          <Td cell="hide-narrow">넓은 화면만</Td>
        </Tr>
      </tbody>
    </Table>
  ),
};

/** Comfortable rows for small lists; two-line content can still grow. */
export const Comfortable: Story = {
  render: () => <Table label="편안한 목록" density="comfortable"><thead><tr><Th>이름</Th><Th>주소</Th></tr></thead><tbody>
    <Tr current><Td cell="primary"><a href="#seoul">서울 운영 노드</a></Td><Td label="주소">seoul.example.test</Td></Tr>
    <Tr><Td cell="primary" secondary="서울 데이터센터"><a href="#busan">부산 운영 노드</a></Td><Td label="주소">busan.example.test</Td></Tr>
  </tbody></Table>,
};
