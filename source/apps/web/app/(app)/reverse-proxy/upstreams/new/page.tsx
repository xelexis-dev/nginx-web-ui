import layout from "@/components/page-layout.module.css";
import Link from "next/link";
import { PageHeader, linkClassName } from "@repo/ui";
import { readPageView } from "@/lib/api-page";
import type { ControlPermissions } from "@/components/reverse-proxy/control-view";
import { UpstreamForm } from "@/components/upstreams/upstream-form";

export const runtime = "nodejs";

/** Create page for an upstream. On success `createUpstream` redirects to the new upstream's detail page. */
export default async function NewUpstreamPage() {
  const { runtimeDns } = await readPageView<ControlPermissions>(
    "/reverse-proxy/upstreams/new",
  );

  return (
    <section className={layout.page}>
      <PageHeader
        eyebrow={
          <Link
            href={"/reverse-proxy/upstreams"}
            className={linkClassName({ variant: "back" })}
          >
            {"Upstreams"}
          </Link>
        }
        title="새 Upstream"
        info="새 upstream 그룹을 등록합니다."
      />

      <UpstreamForm
        runtimeDns={runtimeDns}
        action={{ id: "upstreams.createUpstream", args: [] }}
        cancelHref="/reverse-proxy/upstreams"
      />
    </section>
  );
}
