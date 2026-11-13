import Link from "next/link";
import { SectionNav } from "@repo/ui";
import { PROXY_TABS } from "@/app/(app)/nav-links";
import { readPageView } from "@/lib/api-page";
import { ServerList } from "@/components/reverse-proxy/server-list";
import { readLocalCertificateSummaries } from "@/components/reverse-proxy/certificate-read";
import {
  certificateIndex,
  pendingIndex,
  toOpsRow,
} from "@/components/reverse-proxy/server-status";
import type { ServerListView } from "@/components/reverse-proxy/control-view";

export const runtime = "nodejs";

export default async function ReverseProxyPage() {
  const [view, certificates] = await Promise.all([
    readPageView<ServerListView>("/reverse-proxy"),
    readLocalCertificateSummaries(),
  ]);
  const names = new Map(
    view.upstreams.map((upstream) => [upstream.id, upstream.name]),
  );
  const certs = certificateIndex(certificates);
  const pending = pendingIndex(view.pending.diff);
  const items = view.servers.map((server) =>
    toOpsRow(server, names, certs, pending),
  );
  return (
    <ServerList
      navigation={
        <SectionNav items={PROXY_TABS} currentPath="/reverse-proxy" label="프록시 구성" renderLink={link => <Link {...link} />} />
      }
      data={{
        items,
        certificatesUnknown: certs === null,
        canApply: view.canApply,
        pendingCount: view.pending.count,
        pendingHasBaseline: view.pending.hasBaseline === true,
        pendingDiff: view.pending.diff,
      }}
      basePath="/reverse-proxy"
      write={
        view.canWrite
          ? {
              newHref: "/reverse-proxy/new",
              templatesHref: "/reverse-proxy/templates",
              toggleAction: { id: "reverse-proxy.toggleServer", args: [] },
            }
          : null
      }
    />
  );
}
