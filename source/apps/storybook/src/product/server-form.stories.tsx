import type { ComponentProps } from "react";
import { NodeErrorCard } from "@/components/node-error";
import layout from "@/components/page-layout.module.css";
// Real category-card server form; actions are local previews and never save product data.
import { expect, userEvent, within } from "storybook/test";
import { newLocationInput } from "@/components/reverse-proxy/server-form-vm";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { PageHeader } from "@repo/ui";
import {
  ToastProvider,
  ToastViewport,
} from "@/components/notifications/toast-provider";
import { ServerForm } from "@/components/reverse-proxy/server-form";
import { CERTIFICATES, FORM_VALUES, UPSTREAMS } from "./fixtures";
import { ProductFrame, productViewports } from "./frame";

const meta = {
  id: "product-operations-server-form",
  title: "Product/Operations/서버 수정",
  component: ServerForm,
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  decorators: [
    (Story) => (
      <ToastProvider>
        <ToastViewport />
        <Story />
      </ToastProvider>
    ),
    (Story, context) => {
      const args = context.args;
      const root =
        args.target?.kind === "node"
          ? `/nodes/${args.target.id}/reverse-proxy`
          : "/reverse-proxy";
      return (
        <ProductFrame
          path={args.defaultValues ? `${root}/srv-api/edit` : `${root}/new`}
          label={args.defaultValues ? "서버 수정" : "새 서버"}
          role={context.parameters.offline ? "viewer" : "editor"}
          storyId={context.id}
          viewport={undefined}
          available={!context.parameters.offline}
          nodeName={args.nodeName}
        >
          <Story />
        </ProductFrame>
      );
    },
  ],
  // Composed as the routes compose it (app/(app)/reverse-proxy/[id]/edit and /new): header, then the form.
  render: (args) => (
    <section className={layout.page}>
      <PageHeader
        title={args.defaultValues ? "서버 수정" : "새 서버"}
        info={
          args.defaultValues ? undefined : "새 리버스 프록시 서버를 등록합니다."
        }
      />
      <ServerForm {...args} />
    </section>
  ),
  args: {
    action: { id: "reverse-proxy.updateServer", args: [] },
    upstreams: UPSTREAMS,
    certificates: CERTIFICATES,
    runtimeDns: { available: true, reason: null },
    defaultValues: FORM_VALUES,
    cancelHref: "/reverse-proxy/srv-api",
  },
} satisfies Meta<typeof ServerForm>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Edit: Story = { name: "기존 단일 대상 · 저장 시 자동 전환" };
export const New: Story = {
  name: "새 서버",
  args: { defaultValues: undefined, cancelHref: "/reverse-proxy", action: { id: "reverse-proxy.createServer", args: [] } },
};
export const ValidationErrors: Story = {
  name: "입력 오류(저장을 눌러 보기)",
  args: {
    action: async () => ({
      ok: false,
      fieldErrors: {
        serverName: "도메인 형식이 아닙니다.",
        listen: "3001번은 관리용 포트라 사용할 수 없습니다.",
        certificateId: "인증서가 이 도메인을 포함하지 않습니다.",
      },
    }),
  },
};
export const Mobile: Story = {
  name: "서버 수정 · 390",
  globals: { viewport: { value: "p390", isRotated: false } },
};

export const Routes: Story = {
  name: "여러 경로 · 재시도 켜기",
  args: {
    defaultValues: {
      ...FORM_VALUES,
      locations: [
        {
          ...newLocationInput(),
          upstreamId: "up-api",
          retryOtherInstances: true,
        },
        {
          ...newLocationInput("/assets/"),
          mode: "direct",
          proxyPass: "http://assets.example.test:3000",
          resolveAtRuntime: true,
        },
      ],
    },
  },
};
export const ConversionSaved: Story = {
  name: "단일 대상 전환 · 저장 성공 안내",
  args: { action: async () => ({ ok: true }) },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("button", { name: /^저장$/ }));
    await expect(
      canvas.findByText(
        /저장했습니다 — 이 서버를 경로별 형식으로 전환했습니다/,
      ),
    ).resolves.toBeVisible();
  },
};
export const RouteValidationErrors: Story = {
  name: "경로 오류 · 입력 유지",
  args: {
    defaultValues: {
      ...FORM_VALUES,
      locations: [
        { ...newLocationInput(), upstreamId: "up-api" },
        {
          ...newLocationInput("/assets/"),
          mode: "direct",
          proxyPass: "http://assets.example.test:3000",
        },
      ],
    },
    action: async () => ({
      ok: false,
      fieldErrors: {
        "locations.1.proxyPass": "두 번째 경로의 대상 주소를 확인하세요.",
      },
    }),
  },
};

const NODE_TARGET = {
  kind: "node",
  id: "node-a",
  name: "서울 운영 노드",
} as const;
export const NodeEdit: Story = {
  args: {
    target: NODE_TARGET,
    nodeName: NODE_TARGET.name,
    cancelHref: "/nodes/node-a/reverse-proxy/srv-api",
    action: { id: "node.updateNodeServer", args: ["node-a"] },
  },
};
export const NodeNew: Story = {
  args: {
    ...NodeEdit.args,
    defaultValues: undefined,
    cancelHref: "/nodes/node-a/reverse-proxy",
    action: { id: "node.createNodeServer", args: ["node-a"] },
  },
};
const offlineRender = (args: ComponentProps<typeof ServerForm>) => (
  <section className={layout.page}>
    <PageHeader title={args.defaultValues ? "서버 수정" : "새 서버"} />
    <NodeErrorCard
      nodeId="node-a"
      retryHref={
        args.defaultValues
          ? "/nodes/node-a/reverse-proxy/srv-api/edit"
          : "/nodes/node-a/reverse-proxy/new"
      }
      listHref="/nodes/node-a/reverse-proxy"
      failure={{ kind: "unreachable", message: "노드에 연결할 수 없습니다." }}
    />
  </section>
);
export const NodeEditOffline: Story = {
  ...NodeEdit,
  parameters: { offline: true },
  render: offlineRender,
};
export const NodeNewOffline: Story = {
  ...NodeNew,
  parameters: { offline: true },
  render: offlineRender,
};
export const MobileRailParity: Story = {
  ...NodeEdit,
  args: {
    ...NodeEdit.args,
    action: async () => ({
      ok: false,
      error: "저장하지 못했습니다.",
      fieldErrors: { serverName: "도메인 형식이 아닙니다." },
    }),
  },
};

export const MobileRailGeneralError: Story = {
  ...NodeEdit,
  args: {
    ...NodeEdit.args,
    action: async () => ({ ok: false, error: "저장하지 못했습니다." }),
  },
};

// Shared local/node and pre-2.0 payload cases for the layout migration qualification.
export const LegacyNode: Story = {
  ...NodeEdit,
  args: { ...NodeEdit.args, legacyNode: true, nodeVersionFamily: "1.19" },
};

export const AccessValidation: Story = {
  args: {
    defaultValues: {
      ...FORM_VALUES,
      realIp: {
        preset: "custom",
        trustedRanges: ["203.0.113.0/24"],
        header: "X-Forwarded-For",
        recursive: false,
      },
    },
    action: async () => ({
      ok: false,
      fieldErrors: { realIp: "신뢰 대역을 확인하세요." },
    }),
  },
};
export const CacheValidation: Story = {
  args: {
    action: async () => ({
      ok: false,
      fieldErrors: { cache: "캐시 설정을 확인하세요." },
    }),
  },
};
export const GzipValidation: Story = {
  args: {
    action: async () => ({
      ok: false,
      fieldErrors: { gzip: "최소 응답 크기를 확인하세요." },
    }),
  },
};
export const DirectiveValidation: Story = {
  args: {
    action: async () => ({
      ok: false,
      fieldErrors: { serverDirectives: "허용된 디렉티브인지 확인하세요." },
    }),
  },
};
export const TlsValidation: Story = {
  args: {
    action: async () => ({
      ok: false,
      fieldErrors: { tlsOptions: "TLS 설정을 확인하세요." },
    }),
  },
};
export const PendingSave: Story = {
  args: {
    action: async () =>
      new Promise((resolve) =>
        setTimeout(
          () => resolve({ ok: false, error: "저장하지 못했습니다." }),
          1500,
        ),
      ),
  },
};
export const IpValidation: Story = {
  args: {
    defaultValues: {
      ...FORM_VALUES,
      ipAccess: { mode: "whitelist", entries: ["203.0.113.0/24"] },
    },
    action: async () => ({
      ok: false,
      fieldErrors: { ipAccess: "IP 규칙을 확인하세요." },
    }),
  },
};
export const TlsPortValidation: Story = {
  args: {
    action: async () => ({
      ok: false,
      fieldErrors: { tlsPort: "HTTPS 포트를 확인하세요." },
    }),
  },
};
export const RoutePathValidation: Story = {
  args: {
    ...RouteValidationErrors.args,
    action: async () => ({
      ok: false,
      fieldErrors: { "locations.1.path": "두 번째 경로를 확인하세요." },
    }),
  },
};
export const LocationsValidation: Story = {
  args: {
    ...Routes.args,
    action: async () => ({
      ok: false,
      fieldErrors: { locations: "경로 설정을 확인하세요." },
    }),
  },
};
