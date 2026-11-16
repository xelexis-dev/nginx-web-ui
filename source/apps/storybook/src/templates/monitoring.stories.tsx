import type { Meta, StoryObj } from "@storybook/react-vite";
import {
  Banner,
  CodePane,
  EmptyState,
  KpiRow,
  PageHeader,
  Surface,
} from "@repo/ui";
import styles from "./templates.module.css";
function Monitoring({
  empty = false,
  partial = false,
}: {
  empty?: boolean;
  partial?: boolean;
}) {
  return (
    <main className={styles.page} style={{ maxWidth: 1200, margin: "0 auto" }}>
      <PageHeader title="접속 지표" description="최근 5분 · 예시 데이터" />
      {partial ? (
        <Banner tone="warning" title="일부 데이터를 확인할 수 없습니다." />
      ) : null}
      <KpiRow
        label="현재 연결"
        items={[
          {
            label: "연결 수",
            value: empty ? "—" : "128",
            note: empty ? "수집된 표본이 없습니다." : "nginx 상태 기준",
          },
        ]}
      />
      <Surface title="접속 로그">
        {empty ? (
          <EmptyState description="수집된 로그가 없습니다." />
        ) : (
          <CodePane
            title="접속 기록"
            code={"GET /health 200\nGET /api/ui/v1/overview 200"}
            copyLabel={false}
          />
        )}
      </Surface>
    </main>
  );
}
const meta = {
  id: "templates-monitoring",
  title: "Templates/모니터링과 로그",
  component: Monitoring,
} satisfies Meta<typeof Monitoring>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Spec: Story = {};
export const Empty: Story = { args: { empty: true } };
export const Partial: Story = { args: { partial: true } };
export const Dark: Story = { globals: { theme: "dark" } };
