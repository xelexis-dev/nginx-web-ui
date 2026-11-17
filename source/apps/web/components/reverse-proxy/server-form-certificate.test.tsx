import { expect, it, vi } from "vitest";
import { renderToStaticMarkup as renderMarkup } from "react-dom/server";
import { ToastProvider } from "@/components/notifications/toast-provider";
import type { ReactNode } from "react";
const renderToStaticMarkup=(node:ReactNode)=>renderMarkup(<ToastProvider>{node}</ToastProvider>);

vi.mock("react", async original => ({
  ...await original<typeof import("react")>(),
  useActionState: (_action: unknown, initial: unknown) => [initial, () => undefined, false],
}));
vi.mock("next/navigation", () => ({ useRouter: () => ({ push: vi.fn(), refresh: vi.fn() }) }));
import { ServerForm } from "./server-form";

it.each([
  ["foo.example.test", false],
  ["FOO.EXAMPLE.TEST.", false],
  ["example.test", true],
  ["foo.bar.example.test", true],
] as const)("the shared local/remote form warns for uncovered host %s: %s", (serverName, warning) => {
  const html = renderToStaticMarkup(<ServerForm
    action={{ id: "reverse-proxy.createServer", args: [] }}
    upstreams={[]} certificates={[{ id: "cert", label: "Wildcard", staging: false, domains: ["*.example.test."] }]}
    defaultValues={{ serverName, listen: 80, upstreamId: "", proxyPass: "http://backend:8080", certificateId: "cert" }} cancelHref="/reverse-proxy"
  />);
  expect(html.includes("경고: server_name 중 인증서 도메인에 없는 이름이 있습니다")).toBe(warning);
});

it.each([
  [undefined, false, false], ["2.0.99", true, false], ["2.1.0", true, true], ["2.1.3", false, false],
] as const)("HTTP/2 node %s certificate %s: enabled control %s", (nodeVersion, certificate, enabled) => {
  const html=renderToStaticMarkup(<ServerForm action={{id:"node.createNodeServer",args:["n"]}} target={{kind:"node",id:"n",name:"node"}} nodeVersion={nodeVersion} upstreams={[]} certificates={[]} cancelHref="/nodes/n/reverse-proxy" defaultValues={{serverName:"a.test",listen:80,upstreamId:"",proxyPass:"http://127.0.0.1:9000",certificateId:certificate ? "cert" : null,tlsOptions:{http2:true}}} />);
  const control=html.slice(html.indexOf('aria-label="HTTP/2"'));
  expect(control.slice(0,control.indexOf('</div>')).includes('disabled')).toBe(!enabled);
  if (nodeVersion?.startsWith("2.1") && !certificate) expect(html).toContain("HTTP/2를 사용하려면 인증서를 선택하세요.");
  if (certificate) expect(html).toContain('&quot;http2&quot;:true');
  else expect(html).not.toContain('name="tlsOptions"');
  if (!nodeVersion || nodeVersion.startsWith("2.0")) expect(html).toContain("이 노드는 HTTP/2 설정을 지원하지 않습니다. 노드를 2.1 이상으로 업데이트하세요.");
});
it("new local servers and templates start with HTTP/2 off", () => {
  const html=renderToStaticMarkup(<ServerForm action={{id:"reverse-proxy.createServer",args:[]}} upstreams={[]} certificates={[]} cancelHref="/reverse-proxy" />);
  expect(html).not.toContain('name="tlsOptions"');
  expect(html).toContain("사용 안 함");
});
