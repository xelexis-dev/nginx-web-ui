import type { Meta, StoryObj } from "@storybook/react-vite";
import {
  AppShell,
  Facts,
  SettingRow,
  SettingRows,
  Card,
  CodePane,
  PageHeader,
  SectionNav,
  Select,
  StatusBadge,
  Surface,
  Table,
  Td,
  Th,
  Tr,
} from "@repo/ui";
import { HeaderActions } from "@/components/operations/header-actions";
import { ProfileMenu } from "@/components/profile-menu";
import { NotificationBellView } from "@/components/notifications/notification-bell-view";
import layout from "@/components/page-layout.module.css";

const nav = [
  {
    id: "main",
    label: "구성",
    items: [
      {
        href: "/reverse-proxy",
        label: "리버스 프록시",
        icon: "route" as const,
      },
      { href: "/stream", label: "Stream", icon: "cable" as const },
    ],
  },
];
function RulesTable({ stacked = false }: { stacked?: boolean }) {
  return (
    <Table
      label={stacked ? "좁은 목록" : "행 구분선"}
      density="comfortable"
      narrow={stacked ? "stacked" : "summary"}
    >
      <thead>
        <Tr>
          <Th>이름</Th>
          <Th>로그인 서버 주소</Th>
          <Th>역할</Th>
        </Tr>
      </thead>
      <tbody>
        {["owner@example.test", "viewer@example.test"].map((name) => (
          <Tr key={name}>
            <Td cell="primary">{name}</Td>
            <Td wrap label="로그인 서버 주소">
              https://login.example.test
            </Td>
            <Td label="역할">
              <StatusBadge tone="neutral" size="sm" dot={false}>
                admin
              </StatusBadge>
            </Td>
          </Tr>
        ))}
      </tbody>
    </Table>
  );
}
function SharedViolations() {
  return (
    <AppShell
      nav={nav}
      currentPath="/reverse-proxy"
      topbarContext="로컬 서버"
      topbarActions={
        <HeaderActions
          canApply
          pendingCount={2}
          onApply={() => {}}
          notifications={
            <NotificationBellView
              items={[]}
              unread={0}
              onOpen={() => {}}
              onNavigate={() => {}}
            />
          }
          profile={
            <ProfileMenu
              email="admin@example.test"
              role="admin"
              version="2.1.2"
              sha="preview"
              fullSha="preview"
            />
          }
        />
      }
    >
      <div className={layout.page}>
        <section data-check="V-07">
          <PageHeader title="리버스 프록시" />
        </section>
        <section data-check="V-01">
          <RulesTable />
        </section>
        <section data-check="flush">
          <Surface variant="boxed" flush title="개요 표">
            <RulesTable />
          </Surface>
        </section>
        <section data-check="V-03">
          <StatusBadge tone="neutral" size="sm" dot={false}>
            admin
          </StatusBadge>{" "}
          <StatusBadge size="sm" variant="solid" tone="error" dot={false}>
            emerg
          </StatusBadge>{" "}
          <StatusBadge size="sm" tone="success">
            정상
          </StatusBadge>
        </section>
        <section data-check="V-04">
          <strong>한 번만</strong>
        </section>
        <section data-check="V-06">
          <Card id="shared-card" title="공급자 수정">
            <div data-check="V-09">
              <CodePane
                title="콜백 주소"
                code="https://nginx.example.test/api/auth/callback/keycloak"
              />
            </div>
          </Card>
        </section>
        <section data-check="standalone">
          <CodePane title="독립 코드" code="server_name app.example.test;" />
        </section>
        <section data-check="V-08">
          <SectionNav
            label="설정 탭"
            currentPath="/one"
            items={[
              { href: "/one", label: "기본 설정" },
              { href: "/two", label: "보안" },
              { href: "/three", label: "알림" },
            ]}
          />
        </section>
        <section data-check="V-23">
          <PageHeader
            title="보안"
            description={
              <>
                기본값은 모두 <strong>해제</strong>이며, 강제로 바꾸면 해당 역할
                사용자는 다음 로그인 시 2FA 등록을 마쳐야 합니다.
              </>
            }
          />
        </section>
        <section data-check="V-23-link">
          <PageHeader
            title="메일 서버"
            description={
              <>
                받을 사람은 <a href="#mail">알림 → 메일</a>에서 정합니다.
              </>
            }
          />
        </section>
        <section data-check="V-23-meta">
          <PageHeader
            title="계정"
            descriptionVariant="meta"
            description={
              <>
                <span>owner@example.test</span>
                <StatusBadge tone="neutral" size="sm" dot={false}>
                  admin
                </StatusBadge>
              </>
            }
          />
        </section>
        <section data-check="V-26">
          <RulesTable stacked />
        </section>
        <section data-check="V-28">
          <Select
            aria-label="프로토콜"
            name="protocol"
            defaultValue="tcp"
            options={[
              { value: "tcp", label: "TCP" },
              { value: "udp", label: "UDP" },
            ]}
          />
        </section>
      </div>
    </AppShell>
  );
}
const meta = {
  title: "UI/Approved shared fixes",
  component: SharedViolations,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof SharedViolations>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Comparison: Story = {};

export const RowDecorations: Story = {
  render: () => (
    <AppShell nav={nav} currentPath="/reverse-proxy" topbarContext="로컬 서버">
      <div className={layout.page}>
        <PageHeader title="행 표시" />
        <section data-check="tags">
          <Facts
            items={[
              {
                term: "범위",
                value: (
                  <>
                    <StatusBadge tone="neutral" size="sm" dot={false}>
                      reverse_proxy:read
                    </StatusBadge>{" "}
                    <StatusBadge tone="neutral" size="sm" dot={false}>
                      reverse_proxy:write
                    </StatusBadge>
                  </>
                ),
              },
              {
                term: "역할",
                value: (
                  <StatusBadge tone="neutral" size="sm" dot={false}>
                    admin
                  </StatusBadge>
                ),
              },
            ]}
          />
          <SettingRows>
            <SettingRow
              label="역할"
              status={
                <StatusBadge tone="neutral" size="sm" dot={false}>
                  admin
                </StatusBadge>
              }
            />
          </SettingRows>
        </section>
        {(["dense", "comfortable"] as const).map((density) => (
          <section key={density} data-check={density}>
            <Table label={density} density={density}>
              <thead>
                <Tr>
                  <Th>이름</Th>
                  <Th>상태</Th>
                </Tr>
              </thead>
              <tbody>
                <Tr current>
                  <Td cell="primary">현재 설정</Td>
                  <Td>적용됨</Td>
                </Tr>
                <Tr pending>
                  <Td cell="primary">대기 설정</Td>
                  <Td>미적용</Td>
                </Tr>
                <Tr>
                  <Td cell="primary">일반 설정</Td>
                  <Td>보관됨</Td>
                </Tr>
              </tbody>
            </Table>
          </section>
        ))}
      </div>
    </AppShell>
  ),
};
