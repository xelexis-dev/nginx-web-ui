import layout from "@/components/page-layout.module.css";
import { PageHeader } from "@repo/ui";
import { readPageView } from "@/lib/api-page";
import { StreamForm } from "@/components/streams/stream-form";
import type { StreamEditView } from "@/components/reverse-proxy/control-view";

export const runtime = "nodejs";

export default async function EditStreamPage({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  const { stream, reservedPorts, runtimeDns } = await readPageView<StreamEditView>("/stream/" + encodeURIComponent(id) + "/edit");
  return (
    <section className={layout.page}>
      <PageHeader title={<>Stream 수정</>} />
      <StreamForm
        runtimeDns={runtimeDns}
        action={{ id: "stream.updateStream", args: [] }}
        defaultValues={{ id: stream.id, version: stream.version, listen: stream.listen, protocol: stream.protocol, proxyPass: stream.proxyPass, resolveAtRuntime: stream.resolveAtRuntime }}
        cancelHref={"/stream/" + stream.id}
        reservedUdpListenPorts={reservedPorts.processUdp ?? []}
        reservedListenPorts={[...reservedPorts.process, ...reservedPorts.http]}
      />
    </section>
  );
}
