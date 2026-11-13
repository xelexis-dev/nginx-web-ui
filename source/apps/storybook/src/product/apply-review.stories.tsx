import { useState } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Button, Modal } from "@repo/ui";
import { PendingChangesBadge } from "@/components/pending-changes-badge";
import { ApplyReview, type ApplyBarResult } from "@/components/apply-bar";
import { ToastProvider, ToastViewport } from "@/components/notifications/toast-provider";
import { productFrame, productViewports } from "./frame";
import type { ConfigDiff } from "@/lib/config-diff";

const diff: ConfigDiff = {
  hasChanges: true, streams: [], upstreams: [],
  servers: Array.from({ length: 20 }, (_, i) => ({
    key: String(i), label: `서버 ${i + 1}`, status: "changed",
    changes: [{ field: "proxy_pass", before: "http://old.test", after: i === 19 ? "http://last-diff-line.test" : "http://new.test" }],
  })),
};
const output = Array.from({ length: 12 }, (_, i) => `nginx: [emerg] 검사 출력 ${i + 1}`).join("\n");
function ApplyPreview({ outcome = "failure", nested = false }: { nested?: boolean; outcome?: "success" | "failure" | "transport" | "node" | "unauthorized" }) {
  const [open, setOpen] = useState(false);
  const result = (): ApplyBarResult => outcome === "success" ? { ok: true, output: "nginx: test is successful" }
    : outcome === "failure" ? { ok: false, output }
    : outcome === "transport" ? { ok: false, error: "요청을 보낼 수 없습니다." }
    : { ok: false, nodeError: { nodeId: "preview-node", retryHref: "/nodes/preview-node/reverse-proxy", listHref: "/nodes", failure: { kind: outcome === "node" ? "unreachable" : "unauthorized", message: output } } };
  return <ToastProvider>
    <Button onClick={() => setOpen(true)}>설정 적용 검토</Button>
    <Modal open={open} onClose={() => setOpen(false)} title="설정 적용">
      {nested ? <PendingChangesBadge count={20} hasBaseline diff={diff} /> : null}
      <ApplyReview target={{ kind: "local" }} targetLabel="로컬 서버" onCancel={() => setOpen(false)}
        canApply pendingCount={20} pendingHasBaseline pendingDiff={diff} applyAction={async () => result()} />
    </Modal>
    <ToastViewport />
  </ToastProvider>;
}
const meta = {
  id: "product-apply-review", title: "Product/설정 적용 검토", component: ApplyPreview,
  tags: ["!autodocs"], parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  decorators: [productFrame(() => ({ path: "/reverse-proxy", label: "설정 적용 검토", role: "editor" }))],
} satisfies Meta<typeof ApplyPreview>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Failure: Story = { args: { outcome: "failure" } };
export const Success: Story = { args: { outcome: "success" } };
export const Transport: Story = { args: { outcome: "transport" } };
export const Node: Story = { args: { outcome: "node" } };
export const Unauthorized: Story = { args: { outcome: "unauthorized" } };

export const NestedFailure: Story = { args: { outcome: "failure", nested: true } };
