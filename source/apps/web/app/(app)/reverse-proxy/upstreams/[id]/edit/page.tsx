import layout from "@/components/page-layout.module.css";
import Link from "next/link";
import { PageHeader, linkClassName } from "@repo/ui";
import { readPageView } from "@/lib/api-page";
import { UpstreamForm } from "@/components/upstreams/upstream-form";
import type { UpstreamItemView } from "@/components/reverse-proxy/control-view";

export const runtime = "nodejs";

export default async function EditUpstreamPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = await params;
  const { upstream, runtimeDns } = await readPageView<UpstreamItemView>(
    "/reverse-proxy/upstreams/" + encodeURIComponent(id) + "/edit",
  );
  const servers = Array.isArray(upstream.servers)
    ? (upstream.servers as string[])
    : [];
  return (
    <section className={layout.page}>
      <PageHeader
        eyebrow={
          <Link
            href={"/reverse-proxy/upstreams/" + upstream.id}
            className={linkClassName({ variant: "back" })}
          >
            {upstream.name}
          </Link>
        }
        title="Upstream 수정"
      />
      <UpstreamForm
        runtimeDns={runtimeDns}
        action={{ id: "upstreams.updateUpstream", args: [] }}
        defaultValues={{
          id: upstream.id,
          version: upstream.version,
          name: upstream.name,
          servers,
          resolveAtRuntime: upstream.resolveAtRuntime,
        }}
        cancelHref={"/reverse-proxy/upstreams/" + upstream.id}
      />
    </section>
  );
}
