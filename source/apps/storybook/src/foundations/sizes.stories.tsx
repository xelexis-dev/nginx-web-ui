import type { Meta, StoryObj } from "@storybook/react-vite";
import { light, resolveToken } from "./tokens";
const sizes = [
  ["--size-row", "조밀한 행"],
  ["--size-row-comfy", "편안한 행"],
  ["--size-ctl-sm", "행 컨트롤"],
  ["--size-kv", "라벨 열"],
  ["--size-setting-status", "설정 상태 열"],
  ["--size-fact-value", "Facts 값 열"],
  ["--size-dialog", "확인 창"],
  ["--size-dialog-review", "검토 창"],
];
function Sizes() {
  return (
    <main style={{ maxWidth: 1200, margin: "0 auto" }}>
      <h1>크기</h1>
      <p>라벨 다음의 공유 정렬선은 --size-kv + --sp-4 = 216px입니다.</p>
      <dl>
        {sizes.map(([token, label]) => (
          <div
            key={token}
            style={{
              borderTop: "var(--bw) solid var(--line)",
              paddingBlock: "var(--sp-4)",
            }}
          >
            <dt>
              {label} · {token} · {resolveToken(token!, light)}
            </dt>
            <dd
              style={{
                margin: 0,
                maxWidth: "100%",
                width: `var(${token})`,
                height: "var(--sp-4)",
                background: "var(--bg-selected)",
              }}
            />
          </div>
        ))}
      </dl>
    </main>
  );
}
const meta = { title: "Foundations/Sizes", component: Sizes } satisfies Meta<
  typeof Sizes
>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Spec: Story = {};
export const Dark: Story = { globals: { theme: "dark" } };
