// Actual product error components, not copied markup. Root-layout/API fallback HTML is
// intentionally verified separately: it must work even when the app CSS/JS cannot load.
import type { Meta, StoryObj } from "@storybook/react-vite";
import { useState } from "react";
import { Banner } from "@repo/ui";
import { ErrorView } from "@/components/error-view";
import { NotFoundView } from "@/components/not-found-view";
import { NodeErrorCard } from "@/components/node-error";
import layout from "@/components/page-layout.module.css";
import { productFrame, productViewports } from "./frame";

function ErrorPreview({ kind }: { kind: "error" | "not-found" | "stale" | "node" }) {
  const [retried, setRetried] = useState(false);
  return <section className={layout.page}>
    {retried ? <Banner tone="info" title="다시 시도 미리보기입니다. 서버 요청은 보내지 않았습니다." role="status" /> : null}
    {kind === "error" ? <ErrorView digest="preview-error-id" onRetry={() => setRetried(true)} homeHref="/" /> : null}
    {kind === "not-found" ? <NotFoundView title="페이지를 찾을 수 없습니다" description="주소를 확인하거나 처음 화면으로 돌아가세요." actions={[{ href: "/", label: "처음으로", primary: true }]} /> : null}
    {kind === "stale" ? <ErrorView staleDeployment homeHref="/" /> : null}
    {kind === "node" ? <NodeErrorCard nodeId="preview-node" retryHref="/nodes/preview-node" listHref="/nodes" failure={{ kind: "unreachable", message: "노드에 연결할 수 없습니다." }} /> : null}
  </section>;
}
const meta = {
  id: "product-errors", title: "Product/오류 화면", component: ErrorPreview, tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  decorators: [productFrame(() => ({ path: "/", label: "오류 화면", role: "editor" }))], args: { kind: "error" },
} satisfies Meta<typeof ErrorPreview>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Error: Story = { name: "화면 조회 실패" };
export const NotFound: Story = { name: "찾을 수 없음", args: { kind: "not-found" } };
export const Stale: Story = { name: "이전 버전 탭", args: { kind: "stale" } };
export const Node: Story = { name: "노드 연결 실패", args: { kind: "node" } };
