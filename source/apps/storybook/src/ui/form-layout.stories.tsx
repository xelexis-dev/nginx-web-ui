import {
  Card,
  FormStack,
  Block,
  Blocks,
  FormBar,
  IconButton,
  Icon,
  Button,
  Checkbox,
  Field,
  FieldGroup,
  FormSection,
  formCardClassName,
  formFieldsClassName,
  SegmentedControl,
  Select,
  TextArea,
  TextInput,
  Switch,
} from "@repo/ui";
import type { Meta, StoryObj } from "@storybook/react-vite";
import styles from "./form-layout.module.css";
import { useId, useState } from "react";

function ServerFormLayout({ withErrors }: { withErrors: boolean }) {
  // Per-instance ids: the Docs page shows both stories on one page.
  const uid = useId();
  const id = (name: string) => `${uid}-${name}`;
  const [redirect, setRedirect] = useState("on");
  return (
    <div className={styles.layout}>
      <div className={styles.sections}>
        <FormSection
          id={id("basics")}

          title="기본"
          description="도메인과 수신 포트"
          columns={2}
        >
          <Field
            label="도메인 (server_name)"
            htmlFor={id("name")}
            required
            hint="공백으로 여러 도메인을 구분합니다."
            error={withErrors ? "도메인을 하나 이상 입력하세요." : undefined}
          >
            <TextInput defaultValue={withErrors ? "" : "app.example.test"} />
          </Field>
          <Field label="수신 포트 (listen)" htmlFor={id("listen")} required>
            <TextInput type="number" defaultValue="80" inputMode="numeric" />
          </Field>
          <Field
            label="메모"
            htmlFor={id("note")}
            hint="이 서버를 다루는 사람에게 남길 말"
            wide
          >
            <TextArea defaultValue="" />
          </Field>
        </FormSection>
        <FormSection
          id={id("target")}

          title="대상"
          description="요청을 보낼 upstream 또는 주소"
          columns={2}
        >
          <Field
            label="upstream"
            htmlFor={id("upstream")}
            required
            error={withErrors ? "upstream을 선택하세요." : undefined}
          >
            <Select
              options={[
                { value: "app_pool", label: "app_pool · 대상 3개" },
                { value: "api_v2", label: "api_v2 · 대상 2개" },
              ]}
              placeholder="upstream 선택"
            />
          </Field>
        </FormSection>
        <FormSection
          id={id("tls")}

          title="TLS"
          description="인증서와 HTTPS 동작"
          columns={2}
        >
          <Field label="HTTPS 포트" htmlFor={id("tls-port")}>
            {(control) => <TextInput {...control} defaultValue="443" />}
          </Field>
          <FieldGroup
            id={id("redirect")}
            legend="HTTP→HTTPS 리다이렉트"
            hint="켜면 기존 listen 포트로 온 요청을 HTTPS로 보냅니다."
          >
            <SegmentedControl
              label="HTTP→HTTPS 리다이렉트"
              value={redirect}
              onChange={setRedirect}
              options={[
                { value: "on", label: "리다이렉트" },
                { value: "off", label: "HTTPS만" },
              ]}
            />
          </FieldGroup>
          <div
            data-wide
            style={{ display: "flex", flexWrap: "wrap", gap: "var(--sp-2)" }}
          >
            <Button type="button">저장</Button>
            <Button type="button" variant="secondary">
              취소
            </Button>
          </div>
        </FormSection>
        <FormSection
          id={id("scopes")}

          title="권한"
          description="여러 체크박스를 한 이름으로 묶는 FieldGroup"
          columns={2}
        >
          <FieldGroup
            id={id("scope")}
            legend="권한"
            required
            hint="고른 권한만 이 토큰으로 할 수 있습니다."
            error={withErrors ? "권한을 하나 이상 고르세요." : undefined}
            wide
            columns={2}
          >
            <Checkbox
              name="scopes"
              value="config:read"
              defaultChecked={!withErrors}
              label="config:read"
              description="설정을 읽습니다"
            />
            <Checkbox
              name="scopes"
              value="nginx:reload"
              label="nginx:reload"
              description="설정을 적용합니다"
            />
            <Checkbox
              name="scopes"
              value="node:pair"
              label="node:pair"
              description="노드를 연결합니다"
              disabled
            />
          </FieldGroup>
        </FormSection>
      </div>
    </div>
  );
}

const meta = {
  title: "UI/FormSection · Field",
  component: Field,
  decorators: [
    (Story) => (
      <div className={styles.preview}>
        <Story />
      </div>
    ),
  ],
  parameters: {
    docs: {
      subtitle: "긴 수정 폼: 한 페이지에 모든 섹션과 옆의 섹션 목차",
      description: {
        component: [
          "- `FormStack`은 카테고리 Card를 한 열로 놓는다. Card 머리에는 보조 동작, 본문에는 12열 필드 격자를 둔다.",
          "- `FormSection`: 제목(포커스 가능, 목차로 이동할 때 키보드도 온다)과 설명이 있는 섹션. 머리줄 아래로 스크롤해 가려지지 않는다. `columns={2}`면 필드가 두 열로 흐른다(간격 20, 768px 이하 한 열). `Field wide`나 `data-wide`가 붙은 자식(긴 입력, 동작 줄, 폼 오류)은 두 열을 다 쓴다. 기본은 한 열이고 본문을 배치하지 않는다(기존 폼은 그대로).",
          "- 입력은 `TextInput`·`TextArea`(브라우저 기본 입력에 앱의 칸 모양), 고르기는 `Select`, 파일은 `FileInput`, 몇 개 중 하나는 `SegmentedControl`.",
          "- `Field`: 이름표와 도움말·오류를 컨트롤에 연결한다. 요소를 넘기면 렌더할 때 `id`와 `aria-describedby`/`aria-invalid`/`aria-required`를 붙이고(원래 있던 설명 id는 합친다), 함수를 넘기면 그 속성을 받아 직접 펼친다. `Select`도 이 속성을 트리거에 전달한다. 도움말이 여러 문단이면 `<p>`를 여러 개 넘긴다(하나의 설명으로 묶인다).",
          "- `FieldGroup`: 여러 컨트롤을 한 이름으로 묶는다(`fieldset` + `legend`). `SegmentedControl` 한 줄, 체크박스 여러 개, 반복 행처럼 이름표 하나에 컨트롤이 하나가 아닐 때 쓴다. 도움말·오류는 묶음 전체를 설명한다(`fieldset`의 `aria-describedby`). 모양은 Field와 같다. `columns`로 선택적인 필드 격자를 지정한다.",
        ].join("\n"),
      },
    },
  },
  args: { label: "도메인", htmlFor: "f", children: <TextInput /> },
} satisfies Meta<typeof Field>;
export default meta;
type Story = StoryObj<typeof meta>;

export const EditForm: Story = {
  render: () => <ServerFormLayout withErrors={false} />,
};

/** 저장을 눌렀는데 입력 오류가 있을 때: 목차에 개수, 필드에 문구와 aria-invalid. */
export const WithErrors: Story = {
  render: () => <ServerFormLayout withErrors />,
};

/** Native forms share the card and field grid without adding a nested form element. */
export const PageForm: Story = {
  name: "페이지 폼 · 카드와 필드 격자",
  render: () => <PageFormExample />,
};

function PageFormExample() {
  const id = useId();
  return (
    <form
      className={formCardClassName({ columns: 2 })}
      onSubmit={(event) => event.preventDefault()}
    >
      <Field label="이름" htmlFor={`${id}-name`}>
        <TextInput name="name" />
      </Field>
      <Field label="대상" htmlFor={`${id}-target`}>
        <TextInput name="target" />
      </Field>
      <div data-wide className={formFieldsClassName({ columns: 1 })}>
        <Field label="설명" htmlFor={`${id}-description`}>
          <TextArea name="description" />
        </Field>
        <Checkbox name="enabled" label="사용" defaultChecked />
      </div>
      <div data-wide>
        <Button type="submit">저장</Button>
      </div>
    </form>
  );
}

function InlineExample() {
  const id = useId();
  const [enabled, setEnabled] = useState(true);
  return (
    <form
      className={formCardClassName()}
      onSubmit={(event) => event.preventDefault()}
    >
      <Field
        layout="inline"
        label="인증서 자동 갱신"
        htmlFor={id}
        hint="만료가 가까워지면 인증서를 확인하고 갱신합니다."
      >
        <Switch
          checked={enabled}
          onCheckedChange={setEnabled}
          label="인증서 자동 갱신"
        />
      </Field>
    </form>
  );
}

export const Inline: Story = { render: () => <InlineExample /> };
function DisabledExample() {
  const id = useId();
  return (
    <form className={formCardClassName({ columns: 2 })}>
      <Field
        label="도메인"
        htmlFor={`${id}-name`}
        required
        hint="현재 권한으로는 바꿀 수 없습니다."
      >
        <TextInput disabled defaultValue="app.example.test" />
      </Field>
      <Field
        label="설명"
        htmlFor={`${id}-note`}
        hint="연결을 복구한 뒤 편집할 수 있습니다."
      >
        <TextArea disabled defaultValue="운영 서버의 설명" />
      </Field>
    </form>
  );
}
export const Disabled: Story = { render: () => <DisabledExample /> };
function LongExample() {
  const id = useId();
  return (
    <FormSection
      id={`${id}-form`}

      title="여러 운영자가 함께 관리하는 리버스 프록시의 원본 연결 설정"
      description="긴 설명과 이름도 작은 화면에서 잘리지 않도록 확인합니다."
      columns={2}
    >
      <Field
        label="서울 데이터센터 운영 서비스가 요청을 전달할 원본 도메인 이름"
        htmlFor={`${id}-name`}
        required
        hint="공백으로 여러 도메인을 구분할 수 있으며 실제로 관리하는 서비스의 주소를 입력합니다."
      >
        <TextInput defaultValue="production.example.test" />
      </Field>
      <FieldGroup
        id={`${id}-group`}
        legend="장애 시 요청을 전달할 대체 원본 연결 대상"
        hint="여러 항목의 이름과 설명을 한 묶음으로 읽습니다."
        columns={2}
        required
      >
        <Checkbox
          label="서울 데이터센터의 기본 원본 연결을 사용합니다"
          defaultChecked
        />
        <Checkbox label="부산 데이터센터의 대체 원본 연결을 사용합니다" />
      </FieldGroup>
    </FormSection>
  );
}
export const LongKorean: Story = { render: () => <LongExample /> };
export const EditFormDark: Story = { ...EditForm, globals: { theme: "dark" } };
export const WithErrorsDark: Story = {
  ...WithErrors,
  globals: { theme: "dark" },
};
export const PageFormDark: Story = { ...PageForm, globals: { theme: "dark" } };
export const InlineDark: Story = { ...Inline, globals: { theme: "dark" } };
export const DisabledDark: Story = { ...Disabled, globals: { theme: "dark" } };
export const LongKoreanDark: Story = {
  ...LongKorean,
  globals: { theme: "dark" },
};

function CardStatesExample() {
  const uid = useId();
  const [enabled, setEnabled] = useState(false);
  const [open, setOpen] = useState(true);
  const [secondOpen, setSecondOpen] = useState(false);
  return (
    <main className={styles.preview}>
      <form onSubmit={(event) => event.preventDefault()}>
        <FormStack>
          <Card
            id={`${uid}-default`}
            title="서버"
            description="도메인과 수신 포트"
          >
            <Field label="도메인" htmlFor={`${uid}-domain`} required span={8}>
              <TextInput name="domain" defaultValue="app.example.test" />
            </Field>
            <Field label="포트" htmlFor={`${uid}-port`} span={4}>
              <TextInput name="port" defaultValue="80" />
            </Field>
            <Field label="메모" htmlFor={`${uid}-memo`} optional span={12}>
              <TextArea name="memo" />
            </Field>
          </Card>
          <Card
            id={`${uid}-action`}
            title="위치"
            description="같은 카테고리 안의 반복 항목"
            actions={
              <Button type="button" variant="secondary">
                위치 추가
              </Button>
            }
          >
            <Blocks>
              <Block
                id={`${uid}-expanded`}
                number={1}
                title="/api"
                tag={<span>프록시</span>}
                summary="http://api.internal:8080"
                expanded={open}
                actions={
                  <>
                    <IconButton size="sm" label="위로 이동" disabled>
                      <Icon name="arrowUp" size={14} />
                    </IconButton>
                    <IconButton size="sm" label="아래로 이동">
                      <Icon name="arrowDown" size={14} />
                    </IconButton>
                    <IconButton
                      size="sm"
                      label={open ? "접기" : "펼치기"}
                      aria-expanded={open}
                      aria-controls={`${uid}-expanded-body`}
                      onClick={() => setOpen(!open)}
                    >
                      <Icon name="chevronDown" size={14} />
                    </IconButton>
                    <IconButton size="sm" label="위치 삭제">
                      <Icon name="trash2" size={14} />
                    </IconButton>
                  </>
                }
              >
                <Field label="보낼 주소" htmlFor={`${uid}-target`}>
                  <TextInput
                    name="target"
                    defaultValue="http://api.internal:8080"
                  />
                </Field>
              </Block>
              <Block
                id={`${uid}-collapsed`}
                number={2}
                title="/assets"
                summary="http://assets.internal"
                expanded={secondOpen}
                actions={
                  <IconButton
                    size="sm"
                    label="두 번째 위치 펼치기"
                    aria-expanded={secondOpen}
                    onClick={() => setSecondOpen(!secondOpen)}
                    aria-controls={`${uid}-collapsed-body`}
                  >
                    <Icon name="chevronDown" size={14} />
                  </IconButton>
                }
              >
                <TextInput name="collapsed-path" defaultValue="/assets" />
              </Block>
              <Block
                id={`${uid}-error-block`}
                number={3}
                title="/"
                summary="보낼 주소를 확인하세요"
                errorCount={1}
              >
                <Field
                  label="보낼 주소"
                  htmlFor={`${uid}-invalid-target`}
                  error="주소를 입력하세요."
                >
                  <TextInput name="invalid-target" />
                </Field>
              </Block>
            </Blocks>
          </Card>
          <Card
            id={`${uid}-off`}
            title="프록시 캐시"
            description="끄면 본문은 숨기고 머리는 유지합니다."
            off={!enabled}
            actions={
              <Switch
                label="프록시 캐시 사용"
                checked={enabled}
                onCheckedChange={setEnabled}
              />
            }
          >
            <Field label="캐시 영역" htmlFor={`${uid}-cache`}>
              <TextInput name="cache" defaultValue="app_cache" />
            </Field>
          </Card>
          <Card id={`${uid}-error`} title="TLS" errorCount={2}>
            <Field
              label="인증서"
              htmlFor={`${uid}-cert`}
              error="인증서를 선택하세요."
            >
              <TextInput name="certificate" />
            </Field>
            <Field
              label="HTTPS 포트"
              htmlFor={`${uid}-https-port`}
              error="포트를 입력하세요."
            >
              <TextInput name="https-port" />
            </Field>
            <Field label="TLS 메모" htmlFor={`${uid}-tls-note`} wide optional>
              <TextInput name="tls-note" />
            </Field>
          </Card>
          <Card id={`${uid}-loading`} title="인증서 정보" loading>
            <p>불러온 정보</p>
          </Card>
          <FormBar note="오류 3개 · 입력한 설정을 확인하세요.">
            <Button type="button" variant="secondary">
              취소
            </Button>
            <Button type="submit">저장</Button>
          </FormBar>
        </FormStack>
      </form>
    </main>
  );
}
export const CardAndBlockStates: Story = {
  render: () => <CardStatesExample />,
};
export const CardAndBlockStatesDark: Story = {
  ...CardAndBlockStates,
  globals: { theme: "dark" },
};

/** Field groups keep their category spacing; fields inside each grid use the field gutter. */
export const FieldGroups: Story = {
  render: () => <Card id="spacing-groups" title="서버 연결">
    <div className={formFieldsClassName()} data-proof-group="first">
      <Field label="도메인" htmlFor="group-domain"><TextInput defaultValue="app.example.test" /></Field>
      <Field label="포트" htmlFor="group-port"><TextInput defaultValue="443" /></Field>
    </div>
    <div className={formFieldsClassName()} data-proof-group="second">
      <Field label="원본 주소" htmlFor="group-origin"><TextInput defaultValue="http://app.internal:8080" /></Field>
      <Field label="연결 제한 시간" htmlFor="group-timeout"><TextInput defaultValue="60s" /></Field>
    </div>
  </Card>,
};
