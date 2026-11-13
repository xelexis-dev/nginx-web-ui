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
