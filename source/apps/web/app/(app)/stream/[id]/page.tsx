import { readPageView } from "@/lib/api-page";
import { StreamDetail } from "@/components/streams/stream-detail";
import type { StreamDetailView } from "@/components/reverse-proxy/control-view";

export const runtime = "nodejs";

export default async function StreamDetailPage({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  const view = await readPageView<StreamDetailView>("/stream/" + encodeURIComponent(id));
  return (
    <StreamDetail
      runtimeDns={view.runtimeDns}
      data={{ ...view.stream, preview: view.preview }}
      basePath="/stream"
      canWrite={view.canWrite}
      toggleAction={{ id: "stream.toggleStream", args: [] }}
      deleteAction={{ id: "stream.deleteStream", args: [] }}
    />
  );
}
