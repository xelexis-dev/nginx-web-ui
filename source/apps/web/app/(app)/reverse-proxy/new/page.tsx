import layout from "@/components/page-layout.module.css";
import Link from "next/link";
import { PageHeader, linkClassName } from "@repo/ui";
import { readPageView } from "@/lib/api-page";
import type { ServerOptionsView } from "@/components/reverse-proxy/control-view";
import { ServerForm } from "@/components/reverse-proxy/server-form";
import { toCertificateOptions } from "@/components/reverse-proxy/server-form-vm";
import { getServerTemplate } from "../server-templates";

export const runtime = "nodejs";

/**
 * Create page for a reverse-proxy server. `?template=<id>`가 있으면 해당
 * 템플릿의 defaults로 폼 전체를 프리필한다(무효 id는 조용히 무시 — 빈 폼).
 * On success `createServer` redirects to the new server's detail page.
 */
export default async function NewServerPage({
  searchParams,
}: {
  searchParams: Promise<{ template?: string }>;
}) {
  const view = await readPageView<ServerOptionsView>("/reverse-proxy/new");

  const { template: templateId } = await searchParams;
  const template = templateId ? getServerTemplate(templateId) : undefined;
  const templateDefaults = template
    ? {
        serverName: "",
        listen: template.defaults.listen,
        upstreamId: null,
        proxyPass: template.defaults.proxyPass ?? null,
        locations: template.defaults.locations,
        serverDirectives: template.defaults.serverDirectives,
        locationDirectives: template.defaults.locationDirectives,
      }
    : undefined;

  const certificates = toCertificateOptions(view.certificates);

  return (
    <section className={layout.page}>
      <PageHeader
        title="새 서버"
        info={!template ? "새 리버스 프록시 서버를 등록합니다." : undefined}
        description={
          template ? (
            <>
              템플릿 &ldquo;{template.name}&rdquo; 적용됨 — 대상
              주소(proxy_pass)를 환경에 맞게 수정하세요.
            </>
          ) : (
            <>
              <Link href="/reverse-proxy/templates" className={linkClassName()}>템플릿 보기</Link>
            </>
          )
        }
      />

      <ServerForm
        runtimeDns={view.runtimeDns}
        action={{ id: "reverse-proxy.createServer", args: [] }}
        upstreams={view.upstreams.map((u) => ({
          id: u.id,
          name: u.name,
          resolveAtRuntime: u.resolveAtRuntime,
        }))}
        certificates={certificates}
        defaultValues={templateDefaults}
        cancelHref="/reverse-proxy"
      />
    </section>
  );
}
