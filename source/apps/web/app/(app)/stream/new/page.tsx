import layout from "@/components/page-layout.module.css";
import { PageHeader } from "@repo/ui";
import { readPageView } from "@/lib/api-page";
import type { StreamOptionsView } from "@/components/reverse-proxy/control-view";
import { StreamForm } from "@/components/streams/stream-form";

export const runtime = "nodejs";

/** Create page for an L4 stream. On success `createStream` redirects to the new stream's detail page. */
export default async function NewStreamPage() {
  const { reservedPorts, runtimeDns } = await readPageView<StreamOptionsView>("/stream/new");

  return (
    <section className={layout.page}>
      <PageHeader title={<>새 Stream</>} info={<>새 L4 (TCP/UDP) stream 서버를 등록합니다.</>} />

      {/* 로컬 인스턴스라 예약 포트의 실제 숫자를 안다 — 폼이 그 숫자를 그대로 보여준다. */}
      <StreamForm
        runtimeDns={runtimeDns}
        action={{ id: "stream.createStream", args: [] }}
        cancelHref="/stream"
        reservedUdpListenPorts={reservedPorts.processUdp ?? []}
        reservedListenPorts={[...reservedPorts.process, ...reservedPorts.http]}
      />
    </section>
  );
}
