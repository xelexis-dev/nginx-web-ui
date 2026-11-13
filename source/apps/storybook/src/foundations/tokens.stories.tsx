import { Button, Checkbox, SegmentedControl, Switch } from "@repo/ui";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Swatches } from "./components";
import { token } from "./tokens";
import { renderToStaticMarkup } from "react-dom/server.browser";
import GlobalError from "@/app/global-error";
import "./foundations.css";

const typeRoles = ["xs", "sm", "md", "lg", "xl", "2xl"];

function TokenRoles() {
  return (
    <main className="nwui-found-role-page">
      <h1>공통 토큰</h1>
      <p>같은 이름을 유지하면서 라이트·다크의 역할을 나눕니다.</p>
      <h2>글자 역할</h2>
      <div className="nwui-found-type">
        {typeRoles.map((role) => (
          <div className="nwui-found-type-row" key={role}>
            <span className="nwui-found-type-meta">{role} · {token(`--fs-${role}`)}</span>
            <span className="nwui-found-type-sample" style={{ fontSize: `var(--fs-${role})` }}>리버스 프록시 설정을 확인하세요</span>
          </div>
        ))}
      </div>
      <h2>주요 동작과 상태</h2>
      <div className="nwui-found-role-controls">
        <Button>설정 적용</Button>
        <Button variant="danger">삭제</Button>
        <Button variant="secondary">미리보기</Button>
        <Switch label="서버 활성" checked onCheckedChange={() => undefined} />
        <Checkbox label="설정 확인" defaultChecked />
      </div>
      <SegmentedControl label="전송 방식" options={[{ value: "http", label: "HTTP" }, { value: "stream", label: "Stream" }]} value="http" onChange={() => undefined} />
      <h2>코드·로그·알림</h2>
      <div className="nwui-found-role-code">
        <p>upstream app_pool</p>
        <p className="nwui-found-role-add">+ server app:3000;</p>
        <p className="nwui-found-role-remove">− server legacy:3000;</p>
        <p className="nwui-found-role-warn">경고 · 응답이 지연되고 있습니다</p>
      </div>
      <div className="nwui-found-role-toast" role="status">설정을 적용했습니다</div>
      <h2>같은 테마 안에서 해석한 별칭</h2>
      <Swatches rows={[
        ["--accent", "주요 동작"],
        ["--text-2", "보조 글자"],
        ["--bg-sunken", "코드 바탕"],
        ["--bg-raised", "알림 바탕"],
        ["--accent", "대상 표시"],
      ]} />
    </main>
  );
}

const meta = { title: "Foundations/Token roles", component: TokenRoles, tags: ["!autodocs"] } satisfies Meta<typeof TokenRoles>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Light: Story = { globals: { theme: "light" } };
export const Dark: Story = { globals: { theme: "dark" } };

// Client-root error component only: this story does not simulate server error routing.
function GlobalErrorPreview() {
  const html = renderToStaticMarkup(<GlobalError error={Object.assign(new Error("preview"), { digest: "preview-error-id" })} retry={() => undefined} />);
  return <iframe title="독립 500 화면" srcDoc={`<!doctype html>${html}`} style={{ width: "100%", height: "100dvh", border: 0 }} />;
}
export const GlobalErrorLight: Story = { render: () => <GlobalErrorPreview />, globals: { theme: "light" } };
export const GlobalErrorDark: Story = { render: () => <GlobalErrorPreview />, globals: { theme: "dark" } };

// Foundation specimens exercise the scale before component and screen packages land.
export const Rollout: Story = {
  render: () => (
    <main className="nwui-found-rollout">
      <h1>토큰 검증 · Precision Instrument</h1>
      <div className="nwui-found-rollout-cols" data-rollout-section-stack>
        <section className="nwui-found-rollout-card">
          <h2>글자와 상태</h2>
          {["text-1", "text-2", "text-3", "accent-text", "success-text", "warning-text", "danger-text", "info-text"].map((name) => (
            <span key={name} data-rollout-color={`--${name}`} style={{ color: `var(--${name})` }}>{name} · 설정 상태</span>
          ))}
          <div className="nwui-found-rollout-form" data-rollout-form-grid>
            <span>server_name</span><span>listen</span>
          </div>
        </section>
        <section className="nwui-found-rollout-card">
          <h2>표시와 동작</h2>
          {["success", "warning", "danger", "info", "border-strong", "focus"].map((name) => (
            <span key={name}><i data-rollout-dot={`--${name}`} style={{ backgroundColor: `var(--${name})` }} /> {name}</span>
          ))}
          <button className="nwui-found-rollout-primary" type="button">설정 저장</button>
          <button className="nwui-found-rollout-primary" type="button" disabled>저장 불가</button>
        </section>
      </div>
    </main>
  ),
};
