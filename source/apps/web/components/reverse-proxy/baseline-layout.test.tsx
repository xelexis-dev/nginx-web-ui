import { expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { ServerTable } from "./server-table";
import { UpstreamList } from "../upstreams/upstream-list";
import { StreamList } from "../streams/stream-list";
import { CertificateList } from "../certificates/certificate-list";

vi.mock("next/navigation", () => ({ useRouter: () => ({ refresh: vi.fn(), push: vi.fn() }) }));

it.each(["", "/nodes/node-1"])("keeps entity list semantics on the %s target when empty", (prefix) => {
  for (const [page, columns, message] of [
    [<ServerTable basePath={`${prefix}/reverse-proxy`} servers={[]} write={null} />, 5, "등록된 서버가 없습니다"],
    [<UpstreamList basePath={`${prefix}/reverse-proxy/upstreams`} data={{ items: [], canWrite: false }} />, 2, "등록된 upstream이 없습니다"],
    [<StreamList basePath={`${prefix}/stream`} data={{ items: [], canWrite: false }} />, 4, "등록된 stream이 없습니다"],
    [<CertificateList basePath={`${prefix}/certificates`} data={{ items: [] }} write={null} />, 4, "발급된 인증서가 없습니다"],
  ] as const) {
    const html = renderToStaticMarkup(page);
    expect(html).toContain('data-density="comfortable"');
    expect(html).toContain('data-narrow="stacked"');
    const body = /<tbody>([\s\S]*?)<\/tbody>/.exec(html)![1]!;
    expect(body.match(/<tr\b/g)).toHaveLength(1);
    expect(body).toContain(`colSpan="${columns}"`);
    expect(body).toContain(message);
    expect(html).not.toContain('nwui-btn-primary');
  }
});
