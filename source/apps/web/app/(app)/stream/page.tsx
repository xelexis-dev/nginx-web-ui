import { readPageView } from "@/lib/api-page";
import { StreamList } from "@/components/streams/stream-list";
import type { StreamListView } from "@/components/reverse-proxy/control-view";

export const runtime = "nodejs";

export default async function StreamPage() {
  const view = await readPageView<StreamListView>("/stream");
  return (
    <StreamList
      data={{ items: view.streams, canWrite: view.canWrite }}
      basePath="/stream"
      reservedPorts={view.reservedPorts}
    />
  );
}
