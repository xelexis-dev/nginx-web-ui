import Link from "next/link";
import { SectionNav } from "@repo/ui";
import { PROXY_TABS } from "@/app/(app)/nav-links";
import { readPageView } from "@/lib/api-page";
import type { ConfigPreview } from "@/lib/config-preview";
import { ServerTemplates } from "@/components/reverse-proxy/server-templates";
import { SERVER_TEMPLATES } from "../server-templates";

export const runtime = "nodejs";

export default async function ServerTemplatesPage() {
  const { canWrite, previews } = await readPageView<{
    canWrite: boolean;
    previews: Record<string, ConfigPreview>;
  }>("/reverse-proxy/templates");
  return (
    <ServerTemplates
      canWrite={canWrite}
      navigation={
        <SectionNav items={PROXY_TABS} currentPath="/reverse-proxy/templates" label="프록시 구성" renderLink={link => <Link {...link} />} />
      }
      items={SERVER_TEMPLATES.map((t) => ({
        id: t.id,
        name: t.name,
        description: t.description,
        tags: t.tags,
        preview: previews[t.id],
      }))}
    />
  );
}
