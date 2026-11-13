import { readPageView } from "@/lib/api-page";
import { UpstreamDetail } from "@/components/upstreams/upstream-detail";
import type { UpstreamItemView } from "@/components/reverse-proxy/control-view";

export const runtime = "nodejs";

export default async function UpstreamDetailPage({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  const view = await readPageView<UpstreamItemView>("/reverse-proxy/upstreams/" + encodeURIComponent(id));
  const { upstream } = view;
  return (
    <UpstreamDetail
      runtimeDns={view.runtimeDns}
      data={{ ...upstream, servers: Array.isArray(upstream.servers) ? upstream.servers as string[] : [] }}
      basePath="/reverse-proxy/upstreams"
      canWrite={view.canWrite}
      deleteAction={{ id: "upstreams.deleteUpstream", args: [] }}
    />
  );
}
