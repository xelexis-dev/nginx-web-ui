import { useEffect, useState } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Button } from "@repo/ui";
import { ApplyDialog } from "@/components/apply-dialog";
import type { ApplyBarResult } from "@/components/apply-bar";
import {
  ToastProvider,
  ToastViewport,
  useToast,
} from "@/components/notifications/toast-provider";
import { productFrame, productViewports } from "./frame";
import type { ConfigDiff } from "@/lib/config-diff";

const longName =
  "edge-cdn.example.test static-assets.example.test media-content-delivery.edge.test";
const added = {
  key: "a",
  label: "app.example.test:80",
  status: "added" as const,
  changes: [],
};
const excluded = {
  key: "x",
  label: "excluded.example.test:3001",
  status: "skipped" as const,
  changes: [],
  reason: "포트 3001은 내부 API가 사용합니다.",
};
const diff: ConfigDiff = {
  hasChanges: true,
  streams: [],
  upstreams: [],
  servers: [
    ...Array.from({ length: 12 }, (_, i) => ({
      ...added,
      key: String(i),
      label: `${i === 11 ? longName : `app-${i}.example.test`}:80`,
    })),
    {
      key: "c1",
      label: "api.example.test:80",
      status: "changed",
      changes: [
        {
          field: "config",
          before: {
            locations: [{ path: "/", proxyPass: "http://old.test" }],
            tlsPort: 443,
            redirectHttp: false,
          },
          after: {
            locations: [
              { path: "/", proxyPass: "http://new.test" },
              { path: "/api", proxyPass: "http://api.test" },
              { path: "/assets", proxyPass: "http://assets.test" },
            ],
            tlsPort: 443,
            redirectHttp: true,
          },
        },
      ],
    },
    {
      key: "c2",
      label: "active.example.test:80",
      status: "changed",
      changes: [{ field: "enabled", before: false, after: true }],
    },
    {
      key: "r",
      label: "legacy.example.test:80",
      status: "removed",
      changes: [],
    },
    excluded,
  ],
};
const output = Array.from(
  { length: 12 },
  (_, i) => `nginx: [emerg] 검사 출력 ${i + 1}`,
).join("\n");
type State =
  | "Z"
  | "S"
  | "L"
  | "N"
  | "V"
  | "partial-reopened"
  | "toast-list"
  | "three-toasts"
  | "unknown-empty";
function SeedToasts({ state }: { state: State }) {
  const { toast } = useToast();
  useEffect(() => {
    if (["partial-reopened", "toast-list", "three-toasts"].includes(state))
      toast({
        key: "prior",
        severity: "warning",
        title: "일부 설정을 적용하지 못했습니다",
        body: "표시된 이유를 확인하세요.",
        persistent: true,
      });
    if (state === "three-toasts")
      for (let i = 0; i < 3; i++)
        toast({
          key: `persistent-${i}`,
          severity: "warning",
          title: `확인할 알림 ${i + 1}`,
          persistent: true,
          output,
        });
  }, [state, toast]);
  return null;
}
function ApplyPreview({
  outcome = "failure",
  state = "L",
}: {
  state?: State;
  outcome?:
    "success" | "failure" | "transport" | "node" | "unauthorized" | "partial";
}) {
  const [open, setOpen] = useState(true);
  const result = (): ApplyBarResult =>
    outcome === "success"
      ? { ok: true, output: "nginx: test is successful" }
      : outcome === "partial"
        ? {
            ok: true,
            output: "nginx: test is successful\n적용에서 제외한 설정 1개",
          }
        : outcome === "failure"
          ? { ok: false, output }
          : outcome === "transport"
            ? { ok: false, error: "요청을 보낼 수 없습니다." }
            : {
                ok: false,
                nodeError: {
                  nodeId: "preview-node",
                  retryHref: "/nodes/preview-node/reverse-proxy",
                  listHref: "/nodes",
                  failure: {
                    kind: outcome === "node" ? "unreachable" : "unauthorized",
                    message: output,
                  },
                },
              };
  const pendingDiff =
    state === "Z" || state === "unknown-empty"
      ? { ...diff, hasChanges: false, servers: [] }
      : state === "S"
        ? { ...diff, servers: [added] }
        : state === "partial-reopened"
          ? { ...diff, servers: [excluded] }
          : state === "N"
            ? { ...diff, servers: diff.servers.slice(-3) }
            : diff;
  return (
    <ToastProvider>
      <Button onClick={() => setOpen(true)}>설정 적용 검토</Button>
      <ApplyDialog
        open={open}
        onClose={() => setOpen(false)}
        target={
          state === "N"
            ? { kind: "node", id: "preview-node", name: "edge-seoul-02" }
            : { kind: "local" }
        }
        targetLabel={state === "N" ? "edge-seoul-02" : "로컬 서버"}
        canApply={state !== "V"}
        pendingCount={pendingDiff.servers.length}
        pendingHasBaseline={state !== "unknown-empty"}
        pendingDiff={pendingDiff}
        applyAction={async () => {
          await new Promise((resolve) => setTimeout(resolve, 4000));
          return result();
        }}
      />
      <SeedToasts state={state} />
      <ToastViewport />
    </ToastProvider>
  );
}
const meta = {
  id: "product-apply-review",
  title: "Product/설정 적용 검토",
  component: ApplyPreview,
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  decorators: [
    productFrame(() => ({
      path: "/reverse-proxy",
      label: "설정 적용 검토",
      role: "editor",
    })),
  ],
} satisfies Meta<typeof ApplyPreview>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Failure: Story = { args: { outcome: "failure" } };
export const Success: Story = { args: { outcome: "success" } };
export const Transport: Story = { args: { outcome: "transport" } };
export const Node: Story = { args: { outcome: "node", state: "N" } };
export const Unauthorized: Story = { args: { outcome: "unauthorized" } };
export const Empty: Story = { args: { state: "Z" } };
export const Single: Story = { args: { state: "S" } };
export const Viewer: Story = { args: { state: "V" } };
export const PartialReopened: Story = {
  args: { state: "partial-reopened", outcome: "partial" },
};
export const ToastList: Story = { args: { state: "toast-list" } };
export const ThreeToasts: Story = { args: { state: "three-toasts" } };
export const UnknownEmpty: Story = { args: { state: "unknown-empty" } };
