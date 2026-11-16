import type { Meta, StoryObj } from "@storybook/react-vite";
import { EmptyState, Table, Td, Th, Tr } from "@repo/ui";
const meta = {
  title: "UI/EmptyState",
  component: EmptyState,
  args: {
    title: "등록한 항목이 없습니다",
    description: "새 항목을 등록하면 이곳에서 확인할 수 있습니다.",
  },
  parameters: {
    docs: {
      description: {
        component:
          "조회가 성공했지만 결과가 없을 때 사용합니다. title은 선택이며 간격 4, 자체 세로 여백은 없습니다. 오류나 로딩을 빈 결과로 표시하지 않습니다. 표 안에서는 하나의 spanning cell을 씁니다.",
      },
    },
  },
} satisfies Meta<typeof EmptyState>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Spec: Story = {};
export const States: Story = {
  args: { title: undefined, description: "등록한 패스키가 없습니다." },
};
export const Row: Story = {
  render: () => (
    <Table label="빈 목록">
      <thead>
        <tr>
          <Th>이름</Th>
          <Th>주소</Th>
        </tr>
      </thead>
      <tbody>
        <Tr>
          <Td colSpan={2}>
            <EmptyState description="조건에 맞는 항목이 없습니다." />
          </Td>
        </Tr>
      </tbody>
    </Table>
  ),
};
export const SpecDark: Story = { ...Spec, globals: { theme: "dark" } };
export const StatesDark: Story = { ...States, globals: { theme: "dark" } };
