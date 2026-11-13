import type { Meta, StoryObj } from "@storybook/react-vite";
import routeMeta from "./ui-impl-certificates.stories";

// Keep the real shared local/node compositions; this lane never changes Stream fixtures.
const meta = {
  ...routeMeta,
  render: CertificateRoute,
  id: "product-wp3c-certificates",
  title: "Product/WP3C Certificates",
  args: { ...routeMeta.args, route: "certificateList" },
} satisfies Meta<typeof routeMeta.component>;
export default meta;
type Story = StoryObj<typeof meta>;
export const LocalList: Story = { args: { route: "certificateList" } };
export const LocalDetail: Story = { args: { route: "certificateDetail" } };
export const LocalNew: Story = { args: { route: "certificateNew" } };
export const LocalEdit: Story = { args: { route: "certificateEdit" } };
export const LocalImport: Story = { args: { route: "certificateImport" } };
export const LocalReplace: Story = { args: { route: "certificateReplace" } };
export const NodeList: Story = {
  args: { route: "certificateList", node: true },
};
export const NodeDetail: Story = {
  args: { route: "certificateDetail", node: true, linkedCount: 1 },
};
export const NodeNew: Story = { args: { route: "certificateNew", node: true } };
export const NodeEdit: Story = {
  args: { route: "certificateEdit", node: true },
};
export const NodeImport: Story = {
  args: { route: "certificateImport", node: true },
};
export const NodeReplace: Story = {
  args: { route: "certificateReplace", node: true },
};
export const ViewerList: Story = {
  args: { route: "certificateList", viewer: true },
};
export const ViewerDetail: Story = {
  args: { route: "certificateDetail", viewer: true },
};
export const OfflineNode: Story = {
  args: { route: "certificateDetail", node: true, offline: true },
};

import { CertificateDetail } from "@/components/certificates/certificate-detail";
import { CertificateDetailActions } from "@/components/certificates/certificate-detail-actions";
import { DnsChallengePanel } from "@/components/certificates/dns-challenge-panel";
import type { CertificateDetailVM } from "@/components/certificates/certificate-vm";
const fixture: CertificateDetailVM = {
  id: "cert-api",
  alias: "API 서비스 인증서",
  domains: {
    primary: "api.example.test",
    all: "api.example.test, www.example.test",
    extra: 1,
  },
  source: "acme",
  status: "issued",
  staging: false,
  challengeType: "dns-01",
  dnsProvider: null,
  issuedAt: new Date("2026-10-01T04:00:00Z"),
  expiresAt: new Date("2026-12-30T04:00:00Z"),
  lastAttemptAt: new Date("2026-10-01T04:00:00Z"),
  failureCount: 0,
  lastError: null,
  dnsOrder: null,
  issueLog: [
    {
      at: "2026-10-01 04:00:00",
      level: "info",
      msg: "TXT " + "abcdefghijklmnopqrstuvwxyz0123456789".repeat(12),
    },
  ],
  linkedServers: [],
};
function StateDetail({
  state,
  node = false,
  viewer = false,
}: {
  state: "dns" | "verifying" | "failed" | "issuing" | "manual";
  node?: boolean;
  viewer?: boolean;
}) {
  const base = node ? "/nodes/preview-node/certificates" : "/certificates";
  const waiting = state === "dns" || state === "verifying";
  const source = state === "manual" ? "manual" : "acme";
  const status =
    state === "failed" ? "failed" : state === "issuing" ? "issuing" : "issued";
  const records = [
    {
      domain: "api.example.test",
      recordName: "_acme-challenge.api.example.test",
      recordValue: "fixture-long-txt-" + "abcdef0123456789".repeat(12),
    },
  ];
  const data: CertificateDetailVM = {
    ...fixture,
    source,
    status,
    alias:
      state === "manual" ? "手動-" + "certificate".repeat(12) : fixture.alias,
    lastError: state === "failed" ? "Fixture failed verification" : null,
    failureCount: state === "failed" ? 2 : 0,
    dnsOrder: waiting
      ? {
          phase:
            state === "verifying"
              ? ("verifying" as const)
              : ("awaiting-txt" as const),
          records,
        }
      : null,
  };
  return (
    <CertificateDetail
      basePath={base}
      serverBasePath={
        node ? "/nodes/preview-node/reverse-proxy" : "/reverse-proxy"
      }
      data={data}
      uncovered={[]}
      dnsPanel={
        state === "dns" ? (
          <DnsChallengePanel
            id="cert-api"
            records={records}
            canWrite={!viewer}
            mode="renew"
            cancelPath={base + "/cert-api/cancel"}
            submitAction={{
              id: node
                ? "node.submitNodeDnsChallenge"
                : "certificates.submitDnsChallengeAction",
              args: node ? ["preview-node"] : [],
            }}
          />
        ) : null
      }
      actions={
        viewer ? null : (
          <CertificateDetailActions
            id="cert-api"
            targetName={data.alias ?? ""}
            cancelPath={base + "/cert-api/cancel"}
            editHref={base + "/cert-api/edit"}
            replaceHref={base + "/cert-api/replace"}
            status={status}
            linkedCount={0}
            source={source}
            challengeType="dns-01"
            dnsAutomated={false}
            dnsOrderInFlight={waiting}
            dnsVerifying={state === "verifying"}
            retryAction={{ id: "certificates.retryCertificate", args: [] }}
            renewAction={{ id: "certificates.renewCertificate", args: [] }}
            renewDnsAction={{
              id: "certificates.renewDnsCertificate",
              args: [],
            }}
            deleteAction={{ id: "certificates.deleteCertificate", args: [] }}
          />
        )
      }
    />
  );
}
export const DnsWaiting: Story = {
  args: { route: "certificateDetail" },
  render: (args) => (
    <StateDetail state="dns" node={args.node} viewer={args.viewer} />
  ),
};
export const NodeDnsWaiting: Story = {
  args: { route: "certificateDetail", node: true },
  render: (args) => (
    <StateDetail state="dns" node={args.node} viewer={args.viewer} />
  ),
};
export const ViewerDnsWaiting: Story = {
  args: { route: "certificateDetail", viewer: true },
  render: (args) => (
    <StateDetail state="dns" node={args.node} viewer={args.viewer} />
  ),
};
export const DnsVerifying: Story = {
  args: { route: "certificateDetail" },
  render: (args) => <StateDetail state="verifying" node={args.node} />,
};
export const FailedDetail: Story = {
  args: { route: "certificateDetail" },
  render: (args) => <StateDetail state="failed" node={args.node} />,
};
export const IssuingDetail: Story = {
  args: { route: "certificateDetail" },
  render: (args) => <StateDetail state="issuing" node={args.node} />,
};
export const ManualDetail: Story = {
  args: { route: "certificateDetail" },
  render: (args) => <StateDetail state="manual" node={args.node} />,
};

const explanations = {
  "local-new": (
    <>
      Let&apos;s Encrypt에서 인증서를 발급받습니다. 검증 방식에 따라 필요한
      준비가 다릅니다.
    </>
  ),
  "local-edit": (
    <>
      인증서 ID는 그대로라 <strong>연결된 서버는 그대로 유지</strong>
      됩니다. 저장은 설정만 바꾸며, 실제 재발급은 상세 화면에서 실행합니다.
    </>
  ),
  "local-import": (
    <>
      보유한 PEM 인증서와 개인키를 붙여넣거나 파일로 업로드해 등록합니다.
      도메인·만료일은 인증서에서 자동 추출됩니다.
    </>
  ),
  "local-replace": <>연결된 서버가 있으면 즉시 재적용됩니다.</>,
  "node-new": (
    <>
      이 노드에서 Let&apos;s Encrypt 인증서를 발급받습니다. 검증 방식에 따라
      필요한 준비가 다릅니다.
    </>
  ),
  "node-edit": (
    <>
      인증서 ID는 그대로라 <strong>연결된 서버는 그대로 유지</strong>
      됩니다. 저장은 설정만 바꾸며, 실제 재발급은 상세 화면에서 실행합니다.
    </>
  ),
  "node-import": (
    <>
      보유한 PEM 인증서와 개인키를 붙여넣거나 파일로 업로드해 이 노드에
      등록합니다. 도메인·만료일은 인증서에서 자동 추출됩니다.
    </>
  ),
  "node-replace": <>연결된 서버가 있으면 이 노드에서 즉시 재적용됩니다.</>,
};

import { PageHeader } from "@repo/ui";
import layout from "@/components/page-layout.module.css";
import { CertificateForm } from "@/components/certificates/certificate-form";
import { CertificateEditForm } from "@/components/certificates/certificate-edit-form";
import { ImportForm } from "@/components/certificates/import-form";
function CertificateRoute(args: Parameters<typeof routeMeta.component>[0]) {
  const route =
    args.route === "certificateNew"
      ? "new"
      : args.route === "certificateEdit"
        ? "edit"
        : args.route === "certificateImport"
          ? "import"
          : args.route === "certificateReplace"
            ? "replace"
            : undefined;
  if (!route || args.offline) return <routeMeta.component {...args} />;
  const base = args.node ? "/nodes/preview-node/certificates" : "/certificates";
  const cancelHref = base + (route === "edit" ? "/cert-api" : "");
  const target = args.node
    ? {
        kind: "node" as const,
        id: "preview-node",
        name: "서울 운영 노드 — 긴 이름도 저장 대상을 가리지 않습니다",
      }
    : { kind: "local" as const };
  const actionArgs = args.node ? ["preview-node"] : [];
  const info = explanations[`${args.node ? "node" : "local"}-${route}`];
  return (
    <section className={layout.page}>
      <PageHeader
        title={
          {
            new: "새 인증서",
            edit: "인증서 수정",
            import: "인증서 가져오기",
            replace: "인증서 교체",
          }[route]
        }
      />
      {route === "new" ? (
        <CertificateForm
          info={info}
          target={target}
          action={{
            id: args.node
              ? "node.createNodeCertificate"
              : "certificates.createCertificate",
            args: actionArgs,
          }}
          defaultEmail="operator@example.test"
          cancelHref={cancelHref}
        />
      ) : null}
      {route === "edit" ? (
        <CertificateEditForm
          info={info}
          target={target}
          action={{
            id: args.node
              ? "node.updateNodeCertificate"
              : "certificates.updateCertificate",
            args: actionArgs,
          }}
          id="cert-api"
          cancelHref={cancelHref}
          isIssued
          hasStoredToken
          defaults={{
            alias: "API 서비스 인증서",
            domains: ["api.example.test"],
            email: "operator@example.test",
            staging: false,
            challengeType: "dns-01",
            dnsProvider: "cloudflare",
          }}
        />
      ) : null}
      {route === "import" || route === "replace" ? (
        <ImportForm
          info={info}
          target={target}
          action={{
            id: args.node
              ? route === "replace"
                ? "node.replaceNodeCertificate"
                : "node.importNodeCertificate"
              : route === "replace"
                ? "certificates.replaceCertificate"
                : "certificates.importCertificate",
            args: actionArgs,
          }}
          certId={route === "replace" ? "cert-api" : undefined}
          cancelHref={cancelHref}
        />
      ) : null}
    </section>
  );
}

import { CertificateList } from "@/components/certificates/certificate-list";
export const IssuingList: Story = {
  args: { route: "certificateList", viewer: true },
  render: () => (
    <CertificateList
      basePath="/certificates"
      data={{ items: [{ ...fixture, status: "issuing", linkedCount: 0 }] }}
      write={null}
    />
  ),
};
