import {
  FormStack,
  Card,
  CodePane,
  Field,
  InfoTip,
  Surface,
  TextInput,
} from "@repo/ui";
import type { Meta, StoryObj } from "@storybook/react-vite";

const meta = {
  title: "UI/InfoTip",
  component: InfoTip,
  args: {
    label: "페이지 설명",
    children:
      "페이지와 설정의 배경 설명입니다. 동작의 결과나 입력 조건은 화면에 그대로 둡니다.",
  },
  parameters: {
    layout: "padded",
    docs: {
      description: {
        component:
          "400ms 뒤 나타나는 inverse 툴팁입니다. 포커스·클릭·Esc 동작은 유지하며, 터치에서도 24px 트리거를 사용합니다.",
      },
    },
  },
} satisfies Meta<typeof InfoTip>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = {};
export const LongText: Story = {
  args: {
    children: (
      <>
        {Array.from({ length: 18 }, (_, i) => (
          <p key={i}>
            긴 설명은 화면 안에서 스크롤합니다. 현재 상태와 입력 조건, 이 페이지
            동작의 결과는 정보 아이콘 밖에 보입니다.
          </p>
        ))}
      </>
    ),
  },
};
export const Slots: Story = {
  render: () => (
    <div style={{ display: "grid", gap: 24 }}>
      <Surface
        title="설정"
        info="설정의 배경 설명입니다."
        description="현재 상태"
      >
        <p>본문</p>
      </Surface>
      <FormStack>
        <Card id="options" title="옵션" info="옵션의 배경 설명입니다.">
          <Field
            label="기간"
            htmlFor="days"
            info="기간을 정하는 배경입니다."
            hint="1일 이상"
          >
            <TextInput type="number" />
          </Field>
        </Card>
      </FormStack>
      <CodePane
        title="렌더된 설정"
        info="설정의 배경 설명입니다."
        code="server {}"
      />
    </div>
  ),
};
