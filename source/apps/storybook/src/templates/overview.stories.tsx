import type { Meta, StoryObj } from "@storybook/react-vite";
import {
  Banner,
  Facts,
  KpiRow,
  PageHeader,
  StatusBadge,
  Surface,
  Table,
  Td,
  Th,
  Tr,
} from "@repo/ui";
import { productFrame, productViewports } from "../product/frame";
import { TemplatePreview } from "./preview";
import styles from "./templates.module.css";

function OverviewTemplate({ degraded = false }: { degraded?: boolean }) {
  return (
    <TemplatePreview className={styles.page}>
      <PageHeader
        title="개요"
        description="현재 서버의 설정과 nginx 상태를 확인합니다."
      />
      <div className={styles.kpis}>
        <KpiRow
          label="저장된 설정"
          columns={2}
          items={[
            { label: "서버", value: "12", note: "10개 활성" },
            { label: "Upstreams", value: "4", note: "3개 사용 중" },
          ]}
        />
        <KpiRow
          label="연결과 인증서"
          columns={2}
          items={[
            { label: "현재 연결", value: "128", note: "nginx 상태 기준" },
            {
              label: "인증서",
              value: "8",
              note: "1개 갱신 확인",
              tone: "warning",
            },
          ]}
        />
      </div>
      <div className={styles.overviewMain}>
        <div className={styles.stack}>
          <Surface title="서버 상태" description="저장된 서버 설정" flush>
          <Table label="서버 상태">
            <thead>
              <tr>
                <Th>상태</Th>
                <Th>서버</Th>
                <Th>대상</Th>
              </tr>
            </thead>
            <tbody>
              {[
                "app.example.test",
                "api.example.test",
                "admin.example.test",
              ].map((name) => (
                <Tr key={name}>
                  <Td cell="status">
                    <StatusBadge tone="success">활성</StatusBadge>
                  </Td>
                  <Td cell="primary" wrap>
                    <a href="#server">{name}</a>
                  </Td>
                  <Td cell="meta" label="대상">
                    app_pool
                  </Td>
                </Tr>
              ))}
            </tbody>
          </Table>
          </Surface>
          <Surface title="최근 이벤트">
          <ul className={styles.activity}>
            <li>
              <strong>서버 설정을 저장했습니다.</strong>
              <span>app.example.test · 5분 전</span>
            </li>
            <li>
              <strong>nginx에 설정을 적용했습니다.</strong>
              <span>nginx -t 통과 · 1시간 전</span>
            </li>
          </ul>
          </Surface>
        </div>
        <div className={styles.stack}>
          <Surface title="확인이 필요한 항목">
          {degraded ? (
            <Banner tone="error" title="nginx 상태를 조회할 수 없습니다." />
          ) : (
            <Facts
              items={[
                {
                  term: "nginx",
                  value: <StatusBadge tone="success">연결됨</StatusBadge>,
                },
                { term: "미적용 변경", value: "2건" },
                { term: "만료 예정 인증서", value: "1개" },
              ]}
            />
          )}
          </Surface>
          <Surface title="인증서">
            <Facts
              columns={2}
              items={[
                { term: "전체", value: "8개" },
                { term: "갱신 확인", value: "1개" },
              ]}
            />
          </Surface>
          <Surface title="노드">
            <Facts
              columns={2}
              items={[
                { term: "연결됨", value: "2개" },
                { term: "연결할 수 없음", value: "1개" },
              ]}
            />
          </Surface>
        </div>
      </div>
    </TemplatePreview>
  );
}
const meta = {
  id: "templates-overview",
  title: "Templates/개요",
  component: OverviewTemplate,
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  decorators: [
    productFrame(() => ({ path: "/", label: "개요", role: "editor" })),
  ],
} satisfies Meta<typeof OverviewTemplate>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = {};
export const Degraded: Story = { name: "조회 실패", args: { degraded: true } };
export const Dark: Story = { globals: { theme: "dark" } };
