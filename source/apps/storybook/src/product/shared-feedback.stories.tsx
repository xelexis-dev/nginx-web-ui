import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, waitFor, within } from "storybook/test";
import { Surface } from "@repo/ui";
import { ApplyResult } from "@/components/apply-bar";
import { PendingChangesBadge } from "@/components/pending-changes-badge";
import { RoleBadge } from "@/components/role-badge";
import { IssueLogPanel } from "@/components/certificates/issue-log-panel";

const meta = {
  id: "product-shared-feedback",
  title: "Product/공통 상태와 결과",
  parameters: {
    docs: {
      description: {
        component:
          "업무 문구·동작은 웹 조합 컴포넌트가 담당하고 배너·배지·버튼·대화상자는 @repo/ui를 재사용합니다. API 요청 없이 실제 제품 컴포넌트를 표시합니다.",
      },
    },
  },
} satisfies Meta;
export default meta;
type Story = StoryObj<typeof meta>;

export const ApplySuccess: Story = {
  name: "설정 적용 성공",
  render: () => (
    <ApplyResult
      result={{
        ok: true,
        output:
          "nginx: configuration file /etc/nginx/nginx.conf test is successful",
      }}
    />
  ),
};
export const ApplyFailure: Story = {
  name: "설정 적용 실패",
  render: () => (
    <ApplyResult
      result={{
        ok: false,
        output: 'nginx: [emerg] unknown directive "example_directive"',
      }}
    />
  ),
};
export const Pending: Story = {
  name: "미적용 변경 상세",
  render: () => (
    <PendingChangesBadge
      hasBaseline={true}
      count={1}
      diff={{
        hasChanges: true,
        servers: [
          {
            key: "preview",
            label: "app.example.test:443",
            status: "added",
            changes: [],
          },
        ],
        streams: [],
        upstreams: [],
      }}
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(
      canvas.getByRole("button", { name: "미적용 변경 1건" }),
    );
    const body = within(canvasElement.ownerDocument.body);
    await expect(
      body.getByRole("dialog", { name: "미적용 변경" }),
    ).toBeVisible();
    await userEvent.click(
      within(body.getByRole("dialog", { name: "미적용 변경" })).getByText(
        "닫기",
        { selector: "button" },
      ),
    );
    await waitFor(() =>
      expect(
        canvas.getByRole("button", { name: "미적용 변경 1건" }),
      ).toHaveFocus(),
    );
  },
};
export const Roles: Story = {
  name: "권한 배지",
  render: () => (
    <Surface title="권한">
      <div style={{ display: "flex", gap: 8 }}>
        <RoleBadge role="admin" />
        <RoleBadge role="editor" />
        <RoleBadge role="viewer" />
      </div>
    </Surface>
  ),
};
export const Issuance: Story = {
  name: "인증서 발급 진행",
  render: () => (
    <IssueLogPanel
      live={false}
      entries={[
        {
          at: "2026-10-03 12:00:00",
          level: "info",
          msg: "도메인 소유권 확인을 시작합니다.",
        },
        {
          at: "2026-10-03 12:00:02",
          level: "error",
          msg: "도메인 소유권을 확인하지 못했습니다. DNS 레코드를 확인하세요.",
        },
      ]}
    />
  ),
};
