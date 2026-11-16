import type { Meta, StoryObj } from "@storybook/react-vite";
import {
  Banner,
  Button,
  EmptyState,
  Facts,
  PageHeader,
  SettingRow,
  SettingRows,
  StatusBadge,
  Surface,
  Switch,
  Table,
  Td,
  Th,
  Tr,
} from "@repo/ui";
function RowGrammar() {
  return (
    <main
      style={{
        maxWidth: 1200,
        margin: "0 auto",
        display: "grid",
        gap: "var(--sp-8)",
      }}
    >
      <PageHeader
        title="행의 기준"
        description={
          <>
            라벨 200 <span>간격 16</span>
            <span>공유 정렬선 216</span>
          </>
        }
        status={<StatusBadge tone="success">확정</StatusBadge>}
      />
      <Surface
        title="설정 행"
        tag={
          <StatusBadge tone="neutral" size="sm" dot={false}>
            44 이상
          </StatusBadge>
        }
      >
        <SettingRows>
          <SettingRow
            label="비밀번호"
            status={
              <StatusBadge size="sm" tone="success">
                설정됨
              </StatusBadge>
            }
            detail="로그인에 사용합니다."
            action={
              <Button variant="secondary" size="sm">
                변경
              </Button>
            }
          />
          <SettingRow
            label="외부 로그인"
            status={
              <StatusBadge size="sm" tone="neutral">
                연결 안 됨
              </StatusBadge>
            }
            detail={"아주 긴 한국어 설명 ".repeat(8)}
            action={
              <Button variant="secondary" size="sm">
                연결
              </Button>
            }
            message={<Banner tone="warning" title="연결 상태를 확인하세요." />}
          />
        </SettingRows>
      </Surface>
      <Surface title="설명 있는 Facts">
        <Facts
          items={[
            {
              term: "주소",
              value: "api.example.test",
              note: "등록할 때 확인한 주소입니다.",
            },
            {
              term: "백업",
              value: "동기화 가능",
              note: "현재 상태는 비밀번호 관리자에서 확인하세요.",
            },
          ]}
        />
      </Surface>
      <Surface title="일반 Facts · 상단 정렬">
        <Facts
          items={[
            {
              term: "주소",
              value:
                "주소와 설명이 여러 줄로 표시됩니다.\n기존 첫 줄 정렬을 유지합니다.",
            },
          ]}
        />
      </Surface>
      {(["dense", "comfortable"] as const).map((density) => (
        <Surface
          key={density}
          title={density === "dense" ? "조밀한 표 · 36" : "편안한 표 · 44"}
        >
          <Table
            label={density}
            density={density}
            layout="fixed"
            narrow="stacked"
          >
            <colgroup>
              <col style={{ width: "calc(var(--size-kv) + var(--sp-4))" }} />
              <col />
              <col style={{ width: 72 }} />
            </colgroup>
            <thead>
              <tr>
                <Th>이름</Th>
                <Th>주소</Th>
                <Th>사용</Th>
              </tr>
            </thead>
            <tbody>
              <Tr>
                <Td cell="primary">서울 운영 서버</Td>
                <Td label="주소">api.example.test</Td>
                <Td cell="action" label="사용">
                  <Switch size="sm" label="서울 운영 서버 사용" checked />
                </Td>
              </Tr>
              <Tr>
                <Td cell="primary">부산 운영 서버</Td>
                <Td label="주소">busan.example.test</Td>
                <Td label="사용">설정 안 됨</Td>
              </Tr>
            </tbody>
          </Table>
        </Surface>
      ))}
      <Surface title="빈 표">
        <Table label="빈 목록">
          <thead>
            <tr>
              <Th>이름</Th>
            </tr>
          </thead>
          <tbody>
            <Tr>
              <Td colSpan={1}>
                <EmptyState description="등록한 항목이 없습니다." />
              </Td>
            </Tr>
          </tbody>
        </Table>
      </Surface>
    </main>
  );
}
const meta = {
  title: "Foundations/Row grammar",
  component: RowGrammar,
} satisfies Meta<typeof RowGrammar>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Spec: Story = {};
export const Dark: Story = { globals: { theme: "dark" } };
