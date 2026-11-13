import { readPageView } from "@/lib/api-page";
import { ServerDetail, type ServerDetailVM } from "@/components/reverse-proxy/server-detail";
import { storedConfig } from "@/components/reverse-proxy/server-vm";
import type { ServerDetailView } from "@/components/reverse-proxy/control-view";

export const runtime = "nodejs";

export default async function ServerDetailPage({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  const view = await readPageView<ServerDetailView>("/reverse-proxy/" + encodeURIComponent(id));
  const { server } = view;
  const cert = view.certificates.find(cert => cert.id === server.certificateId);
  const data: ServerDetailVM = {
    ...server,
    preview: view.preview,
    config: storedConfig(server.config),
    upstreams: view.upstreams.map(upstream => ({ ...upstream, servers: Array.isArray(upstream.servers) ? upstream.servers as string[] : [] })),
    cert: cert ? {
      id: cert.id, domains: Array.isArray(cert.domains) ? cert.domains as string[] : [],
      staging: cert.staging, issued: cert.status === "issued", tlsReady: cert.tlsReady,
    } : null,
    selfProxyNotice: view.selfProxyNotice,
    selfProxyUncheckedPaths: view.selfProxyUncheckedPaths,
  };
  return (
    <ServerDetail
      runtimeDns={view.runtimeDns}
      data={data}
      basePath="/reverse-proxy"
      configOrigin="this-instance"
      write={view.canWrite ? {
        editHref: "/reverse-proxy/" + server.id + "/edit",
        toggleAction: { id: "reverse-proxy.toggleServer", args: [] },
        deleteAction: { id: "reverse-proxy.deleteServer", args: [] },
      } : null}
    />
  );
}
