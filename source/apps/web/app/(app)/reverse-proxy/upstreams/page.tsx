import Link from "next/link";
import { SectionNav } from "@repo/ui";
import { PROXY_TABS } from "@/app/(app)/nav-links";
import { readPageView } from "@/lib/api-page";
import { UpstreamList } from "@/components/upstreams/upstream-list";
import type { UpstreamListView } from "@/components/reverse-proxy/control-view";

export const runtime = "nodejs";

export default async function UpstreamsPage() {
  const view = await readPageView<UpstreamListView>("/reverse-proxy/upstreams");
  const items = view.upstreams.map((upstream) => ({
    ...upstream,
    servers: Array.isArray(upstream.servers)
      ? (upstream.servers as string[])
      : [],
  }));
  return (
    <UpstreamList
      navigation={
        <SectionNav items={PROXY_TABS} currentPath="/reverse-proxy/upstreams" label="프록시 구성" renderLink={link => <Link {...link} />} />
      }
      data={{ items, canWrite: view.canWrite }}
      basePath="/reverse-proxy/upstreams"
    />
  );
}
