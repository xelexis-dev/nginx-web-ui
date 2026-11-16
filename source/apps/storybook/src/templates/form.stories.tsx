import { TemplatePreview } from "./preview";
import { FormErrors } from "@/components/form-errors";
// Template: a create/edit page (Storybook only). The contract a form screen follows, built from
// @repo/ui alone: header with a back link, the form's own error when saving failed, then titled
// sections whose fields flow in two columns (Card), and the action row. Saving
// is a preview: the form only shows its states.
import {
  Banner,
  Button,
  Field,
  Card,
  FormStack,
  FormBar,
  formFieldsClassName,
  PageHeader,
  Surface,
  Facts,
  SegmentedControl,
  Select,
  TextArea,
  TextInput,
  buttonClassName,
} from "@repo/ui";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { useId, useState } from "react";
import { productFrame, productViewports } from "../product/frame";
import styles from "./templates.module.css";

function FormTemplate({
  failed,
  remote = false,
}: {
  failed: boolean;
  remote?: boolean;
}) {
  const uid = useId();
  const id = (name: string) => `${uid}-${name}`;
  const [health, setHealth] = useState(true);
  return (
    <TemplatePreview className={styles.page}>
      <PageHeader
        title="새 업스트림"
        description="요청을 나눠 보낼 대상 서버 묶음을 만듭니다. 저장한 뒤 「설정 적용」으로 nginx에 반영합니다."
      />
      <form
        className={styles.stack}
        noValidate
        onSubmit={(event) => event.preventDefault()}
      >
        {failed ? (
          <FormErrors
            errors={{
              name: "같은 이름의 업스트림이 이미 있습니다.",
              servers: "대상을 하나 이상 입력하세요.",
            }}
            targets={{ name: id("name"), servers: id("servers") }}
          />
        ) : null}
        <FormStack>
          <Banner
            tone="info"
            title="저장한 변경은 머리줄의 적용으로 nginx에 반영합니다."
          />
          <Card
            id={id("basics")}
            title="기본"
            description="이름과 부하 분산 방식"
          >
            <div className={formFieldsClassName({ columns: 2 })}>
              <Field
                label="이름"
                htmlFor={id("name")}
                required
                hint="영문, 숫자, _ 만 씁니다."
                error={
                  failed ? "같은 이름의 업스트림이 이미 있습니다." : undefined
                }
              >
                <TextInput
                  name="name"
                  defaultValue={failed ? "app_pool" : ""}
                  autoComplete="off"
                  required
                />
              </Field>
              <Field label="부하 분산" htmlFor={id("balance")}>
                <Select
                  name="balance"
                  defaultValue="round_robin"
                  options={[
                    { value: "round_robin", label: "차례대로 (round-robin)" },
                    {
                      value: "least_conn",
                      label: "연결이 적은 곳 (least_conn)",
                    },
                    { value: "ip_hash", label: "클라이언트 IP 고정 (ip_hash)" },
                  ]}
                />
              </Field>
              <Field
                label="설명"
                htmlFor={id("note")}
                hint="목록에 함께 보입니다."
                wide
              >
                <TextArea name="note" />
              </Field>
            </div>
          </Card>
          <Card
            id={id("targets")}
            title="대상 서버"
            description="한 줄에 하나씩 host:port"
          >
            <div className={formFieldsClassName({ columns: 2 })}>
              <Field
                label="대상"
                htmlFor={id("servers")}
                required
                wide
                error={failed ? "대상을 하나 이상 입력하세요." : undefined}
              >
                <TextArea
                  name="servers"
                  required
                  mono
                  placeholder={"10.0.1.10:3000\n10.0.1.11:3000"}
                />
              </Field>
              <Field label="실패로 볼 횟수" htmlFor={id("fails")}>
                <TextInput
                  name="maxFails"
                  type="number"
                  inputMode="numeric"
                  defaultValue="3"
                />
              </Field>
              <Field label="다시 시도할 때까지(초)" htmlFor={id("timeout")}>
                <TextInput
                  name="failTimeout"
                  type="number"
                  inputMode="numeric"
                  defaultValue="10"
                />
              </Field>
              <div data-wide className={styles.setting}>
                <div className={styles.settingText}>
                  <p className={styles.settingName}>상태 확인</p>
                  <p className={styles.settingNote}>
                    실패한 대상을 잠시 빼고 보냅니다.
                  </p>
                </div>
                <SegmentedControl
                  label="상태 확인"
                  options={[
                    { value: false, label: "사용 안 함" },
                    { value: true, label: "사용" },
                  ]}
                  value={health}
                  onChange={setHealth}
                />
                <input
                  type="hidden"
                  name="healthCheck"
                  value={String(health)}
                />
              </div>
            </div>
          </Card>
        </FormStack>
        <FormBar note={remote ? "저장 대상: 미리보기 노드" : undefined}>
          <a href="#list" className={buttonClassName({ variant: "secondary" })}>
            취소
          </a>
          <Button type="submit">저장</Button>
        </FormBar>
      </form>
    </TemplatePreview>
  );
}

const meta = {
  id: "templates-form",
  title: "Templates/폼",
  component: FormTemplate,
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  decorators: [
    productFrame((args) => ({
      path: args.remote
        ? "/nodes/preview-node/reverse-proxy/upstreams"
        : "/reverse-proxy/upstreams",
      label: "새 업스트림",
      role: "editor",
    })),
  ],
  args: { failed: false },
} satisfies Meta<typeof FormTemplate>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = { name: "폼" };
export const Failed: Story = { name: "저장 실패", args: { failed: true } };
export const Narrow: Story = {
  name: "폼 · 390",
  globals: { viewport: { value: "p390", isRotated: false } },
};

export const Dark: Story = { globals: { theme: "dark" } };

export const Node: Story = {
  name: "노드 폼 · 저장 대상",
  args: { remote: true },
};

export const ProxySummaryRail: Story = {
  name: "프록시 서버 전용 · 요약 레일",
  render: () => (
    <TemplatePreview className={styles.page}>
      <PageHeader
        title="새 서버"
        description="저장한 뒤 설정 적용으로 nginx에 반영합니다."
      />
      <form
        className={styles.formWithRail}
        onSubmit={(event) => event.preventDefault()}
      >
        <FormStack>
          <Banner
            tone="info"
            title="저장한 변경은 머리줄의 적용으로 nginx에 반영합니다."
          />
          <Card id="template-proxy-basics" title="기본">
            <div className={formFieldsClassName({ columns: 2 })}>
              <Field label="도메인" htmlFor="proxy-domain">
                <TextInput id="proxy-domain" defaultValue="app.example.test" />
              </Field>
              <Field label="대상" htmlFor="proxy-target">
                <TextInput id="proxy-target" defaultValue="http://app_pool" />
              </Field>
            </div>
          </Card>
        </FormStack>
        <aside className={styles.summaryRail}>
          <Surface
            title="한눈에 보기"
            description="저장할 설정입니다. 저장한 뒤 「설정 적용」 전까지 nginx에는 반영되지 않습니다."
          >
            <Facts
              items={[
                { term: "도메인", value: "app.example.test" },
                { term: "대상", value: "http://app_pool" },
              ]}
            />
            <div className={styles.actions}>
              <Button type="submit">저장</Button>
              <a
                href="#list"
                className={buttonClassName({ variant: "secondary" })}
              >
                취소
              </a>
            </div>
          </Surface>
        </aside>
      </form>
    </TemplatePreview>
  ),
};
