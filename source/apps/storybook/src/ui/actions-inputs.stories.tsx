import { useState } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import {
  Button,
  Checkbox,
  FileInput,
  FilterChips,
  Icon,
  IconButton,
  InfoTip,
  SearchField,
  SegmentedControl,
  Switch,
  TextArea,
  TextInput,
  TextLink,
} from "@repo/ui";
const stack = {
  display: "grid",
  alignContent: "start",
  gap: "var(--gap-field)",
  minWidth: 0,
} as const;
const heading = {
  margin: 0,
  fontSize: "var(--fs-lg)",
  lineHeight: "var(--lh-lg)",
  fontWeight: "var(--fw-semibold)",
} as const;
const field = {
  display: "grid",
  alignContent: "start",
  gap: "var(--gap-field-inner)",
  minWidth: 0,
  fontSize: "var(--fs-sm)",
  lineHeight: "var(--lh-sm)",
} as const;
function ActionsInputs() {
  const [checked, setChecked] = useState(false);
  const [enabled, setEnabled] = useState(false);
  const [query, setQuery] = useState("");
  const [mode, setMode] = useState("http");
  const [filter, setFilter] = useState("all");
  const [fileName, setFileName] = useState("파일 없음");
  return (
    <main
      style={{
        ...stack,
        maxWidth: "var(--size-content)",
        marginInline: "auto",
        padding: "var(--page-x)",
        gap: "var(--gap-header)",
        color: "var(--text-1)",
        background: "var(--bg-canvas)",
        fontFamily: "var(--font)",
        fontSize: "var(--fs-md)",
        lineHeight: "var(--lh-md)",
      }}
    >
      <h1
        style={{
          ...heading,
          fontSize: "var(--fs-2xl)",
          lineHeight: "var(--lh-2xl)",
        }}
      >
        동작과 입력
      </h1>
      <div
        data-rollout-section-stack
        style={{
          ...stack,
          gridTemplateColumns:
            "repeat(auto-fit, minmax(min(100%, var(--size-pop-min)), 1fr))",
          gap: "var(--gap-cols)",
        }}
      >
        <section style={stack}>
          <h2 style={heading}>
            입력 상태{" "}
            <InfoTip label="입력 설명">
              설정 값의 형식과 동작을 확인합니다.
            </InfoTip>
          </h2>
          <form
            id="wp1a-inputs"
            style={stack}
            onSubmit={(event) => event.preventDefault()}
          >
            <div
              data-rollout-form-grid
              style={{
                ...stack,
                gridTemplateColumns:
                  "repeat(auto-fit, minmax(min(100%, var(--size-pop-min)), 1fr))",
              }}
            >
              <label style={field}>
                도메인
                <TextInput name="domain" defaultValue="app.example.test" />
              </label>
              <label style={field}>
                오류 입력
                <TextInput
                  aria-invalid
                  aria-describedby="wp1a-error"
                  defaultValue="invalid"
                />
                <span
                  id="wp1a-error"
                  style={{
                    color: "var(--danger-text)",
                    fontSize: "var(--fs-xs)",
                    lineHeight: "var(--lh-xs)",
                  }}
                >
                  도메인을 확인하세요.
                </span>
              </label>
              <label style={field}>
                읽기 전용
                <TextInput readOnly defaultValue="nwui-token" />
              </label>
              <label style={field}>
                사용 불가
                <TextInput disabled defaultValue="설정 잠김" />
              </label>
            </div>
            <label style={field}>
              설정 원문
              <TextArea
                mono
                name="config"
                defaultValue={
                  "proxy_read_timeout 60s;\nproxy_send_timeout 60s;"
                }
              />
            </label>
            <label style={field}>
              인증서 파일
              <FileInput
                accept=".pem"
                onChange={(event) =>
                  setFileName(
                    event.currentTarget.files?.[0]?.name ?? "파일 없음",
                  )
                }
              />
              <span role="status">{fileName}</span>
            </label>
            <Checkbox
              name="confirmed"
              value="yes"
              label="설정 확인"
              description="저장할 내용을 확인했습니다."
              checked={checked}
              onChange={(event) => setChecked(event.currentTarget.checked)}
            />
            <Checkbox label="비활성 선택" defaultChecked disabled />
            <Checkbox
              label="일부 선택"
              ref={(input) => {
                if (input) input.indeterminate = true;
              }}
            />
          </form>
        </section>
        <section style={stack}>
          <h2 style={heading}>동작과 선택</h2>
          <div
            style={{
              display: "flex",
              alignItems: "center",
              flexWrap: "wrap",
              gap: "var(--gap-actions)",
            }}
          >
            <Button type="submit" form="wp1a-inputs">
              <Icon name="check" />
              저장
            </Button>
            <Button type="button" variant="secondary">
              취소
            </Button>
            <Button type="button" variant="dangerQuiet">
              <Icon name="trash2" />
              삭제
            </Button>
            <IconButton label="새로고침" size="sm">
              <Icon name="refresh" />
            </IconButton>
            <Button type="button" variant="secondary" size="xs">
              복사
            </Button>
            <Button type="button" variant="secondary" size="lg">
              큰 동작
            </Button>
          </div>
          <Switch
            checked={enabled}
            onCheckedChange={setEnabled}
            label="서버 활성"
            showState
          />
          <Switch checked disabled label="비활성 스위치" />
          <SegmentedControl
            label="전송 방식"
            value={mode}
            onChange={setMode}
            options={[
              { value: "http", label: "HTTP" },
              { value: "tcp", label: "TCP" },
              { value: "udp", label: "UDP" },
            ]}
          />
          <FilterChips
            label="상태 필터"
            value={filter}
            onChange={setFilter}
            options={[
              { value: "all", label: "전체", count: 3 },
              { value: "active", label: "활성", count: 2 },
            ]}
          />
          <SearchField
            label="서버 검색"
            value={query}
            onChange={setQuery}
            placeholder="도메인 검색"
          />
          <TextLink href="#wp1a-inputs">설정으로 이동</TextLink>
        </section>
      </div>
    </main>
  );
}
const meta = {
  title: "UI/Actions and inputs",
  component: ActionsInputs,
  parameters: { layout: "fullscreen" },
  tags: ["!autodocs"],
} satisfies Meta<typeof ActionsInputs>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Rollout: Story = {};
export const Dark: Story = { globals: { theme: "dark" } };
