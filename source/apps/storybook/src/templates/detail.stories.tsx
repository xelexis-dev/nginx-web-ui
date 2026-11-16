import { TemplatePreview } from "./preview";
// Template: a detail page (Storybook only). The contract a detail screen follows, built from
// @repo/ui alone with sample values: header with a back link and the item's actions, a notice when
// there is one, then the item's facts, its related rows and the generated config beside them.
import {
  Banner,
  Button,
  CodePane,
  Facts,
  Icon,
  PageHeader,
  StatusBadge,
  Surface,
  Table,
  Td,
  Th,
  Tr,
  buttonClassName,
} from "@repo/ui";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { productFrame, productViewports } from "../product/frame";
import styles from "./templates.module.css";

const CONFIG =
  "upstream app_pool {\n    least_conn;\n    server 10.0.1.10:3000 max_fails=3 fail_timeout=10s;\n    server 10.0.1.11:3000 max_fails=3 fail_timeout=10s;\n    server 10.0.1.12:3000 backup;\n}\n";

const TARGETS = [
  { id: "t1", address: "10.0.1.10:3000", weight: 1, role: "기본" },
  { id: "t2", address: "10.0.1.11:3000", weight: 1, role: "기본" },
  { id: "t3", address: "10.0.1.12:3000", weight: 1, role: "예비(backup)" },
];

function DetailTemplate({ pending }: { pending: boolean }) {
  return (
    <TemplatePreview className={styles.page}>
      <PageHeader
        eyebrow={
          <a href="#list">
            <Icon name="back" size={14} />
            업스트림
          </a>
        }
        title="app_pool"
        status={<StatusBadge tone="success">사용 중</StatusBadge>}
        description="리버스 프록시 서버 2개가 이 묶음으로 요청을 보냅니다."
        actions={
          <>
            <Button variant="dangerQuiet" data-preview-action>
              삭제
            </Button>
            <a
              href="#edit"
              className={buttonClassName({ variant: "secondary" })}
            >
              수정
            </a>
          </>
        }
      />
      {pending ? (
        <Banner
          tone="warning"
          title="아직 nginx에 적용하지 않은 변경이 있습니다."
        >
          머리줄의 「설정 적용」으로 반영합니다.
        </Banner>
      ) : null}
      <div className={styles.stack}>
        <div className={styles.stack}>
          <Surface title="설정">
            <Facts
              columns={2}
              items={[
                {
                  term: "상태",
                  value: <StatusBadge tone="success">사용 중</StatusBadge>,
                },
                { term: "부하 분산", value: "least_conn" },
                { term: "실패 기준", value: "3회 · 10초", mono: true },
                { term: "마지막 변경", value: "2026. 9. 27. 오전 10:02" },
                {
                  term: "쓰는 서버",
                  value: "app.example.test, admin.example.test",
                  wide: true,
                },
              ]}
            />
          </Surface>
          <Surface
            title="대상 서버"
            description="부하 분산 방식에 따라 요청을 전달할 대상입니다."
            flush
          >
            <Table label="대상 서버">
              <thead>
                <tr>
                  <Th>주소</Th>
                  <Th align="end">가중치</Th>
                  <Th>역할</Th>
                </tr>
              </thead>
              <tbody>
                {TARGETS.map((t) => (
                  <Tr key={t.id}>
                    <Td cell="primary" mono>
                      {t.address}
                    </Td>
                    <Td numeric label="가중치">
                      {t.weight}
                    </Td>
                    <Td label="역할">{t.role}</Td>
                  </Tr>
                ))}
              </tbody>
            </Table>
          </Surface>
        </div>
        <CodePane
          title="생성되는 설정"
          description="저장된 값으로 만든 nginx 설정입니다."
          code={CONFIG}
        />
      </div>
    </TemplatePreview>
  );
}

const meta = {
  id: "templates-detail",
  title: "Templates/상세",
  component: DetailTemplate,
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  decorators: [
    productFrame(() => ({
      path: "/reverse-proxy/upstreams",
      label: "app_pool",
      role: "editor",
    })),
  ],
  args: { pending: false },
} satisfies Meta<typeof DetailTemplate>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = { name: "상세" };
export const Pending: Story = {
  name: "미적용 변경 있음",
  args: { pending: true },
};
export const Narrow: Story = {
  name: "상세 · 390",
  args: { pending: true },
  globals: { viewport: { value: "p390", isRotated: false } },
};

export const Dark: Story = { globals: { theme: "dark" } };
