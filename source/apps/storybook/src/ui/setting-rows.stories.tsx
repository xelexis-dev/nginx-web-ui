import type { Meta, StoryObj } from "@storybook/react-vite";
import { Banner, Button, SettingRow, SettingRows, StatusBadge } from "@repo/ui";
const meta = {
  title: "UI/SettingRows",
  component: SettingRows,
  args: { children: null },
  parameters: {
    docs: {
      description: {
        component:
          "설정 행: 200 라벨 / 176 상태 / 설명 / 동작. 44px 이상, 가운데 정렬. 오류는 행 아래 전체 폭. 좁은 화면은 텍스트 묶음과 동작으로 나눕니다. 레코드 목록에는 Table을 사용합니다.",
      },
    },
  },
  render: () => (
    <SettingRows>
      <SettingRow
        label="비밀번호"
        status={
          <StatusBadge size="sm" tone="success">
            설정됨
          </StatusBadge>
        }
        action={
          <Button size="sm" variant="secondary">
            변경
          </Button>
        }
      />
      <SettingRow
        label="2단계 인증"
        status={
          <StatusBadge size="sm" tone="neutral">
            설정 안 됨
          </StatusBadge>
        }
        detail="로그인할 때 인증 앱의 코드를 한 번 더 확인합니다."
        action={
          <Button size="sm" variant="secondary">
            설정
          </Button>
        }
      />
      <SettingRow
        label="API 접근"
        wideDetail
        detail="API나 자동화 도구가 이 계정의 권한으로 접근할 때 씁니다."
        action={
          <Button size="sm" variant="secondary">
            토큰 관리
          </Button>
        }
      />
    </SettingRows>
  ),
} satisfies Meta<typeof SettingRows>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Spec: Story = {};
export const States: Story = {
  render: () => (
    <SettingRows>
      <SettingRow
        label={"긴 한국어 이름".repeat(10)}
        status={
          <StatusBadge size="sm" tone="warning">
            확인 필요
          </StatusBadge>
        }
        detail="마지막 남은 로그인 방법입니다."
        action={
          <Button size="sm" disabled variant="secondary">
            해제
          </Button>
        }
        message={
          <Banner
            tone="error"
            title="마지막 로그인 방법은 해제할 수 없습니다."
          />
        }
      />
    </SettingRows>
  ),
};
export const SpecDark: Story = { ...Spec, globals: { theme: "dark" } };
export const StatesDark: Story = { ...States, globals: { theme: "dark" } };
