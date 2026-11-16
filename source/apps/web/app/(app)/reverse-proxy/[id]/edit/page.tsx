import layout from "@/components/page-layout.module.css";
import { PageHeader } from "@repo/ui";
import { readPageView } from "@/lib/api-page";
import { ServerForm } from "@/components/reverse-proxy/server-form";
import {
  toCertificateOptions,
  toServerFormValues,
} from "@/components/reverse-proxy/server-form-vm";
import type { ServerEditView } from "@/components/reverse-proxy/control-view";

export const runtime = "nodejs";

export default async function EditServerPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = await params;
  const view = await readPageView<ServerEditView>(
    "/reverse-proxy/" + encodeURIComponent(id) + "/edit",
  );
  const upstreams = view.upstreams.map((upstream) => ({
    id: upstream.id,
    name: upstream.name,
    resolveAtRuntime: upstream.resolveAtRuntime,
  }));
  return (
    <section className={layout.page}>
      <PageHeader
        title="서버 수정"
      />
      <ServerForm
        runtimeDns={view.runtimeDns}
        action={{ id: "reverse-proxy.updateServer", args: [] }}
        upstreams={upstreams}
        certificates={toCertificateOptions(view.certificates)}
        defaultValues={toServerFormValues(view.server, upstreams)}
        cancelHref={"/reverse-proxy/" + view.server.id}
      />
    </section>
  );
}
