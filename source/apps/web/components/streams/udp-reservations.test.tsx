import { beforeEach, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
const mock = vi.hoisted(() => ({ read: vi.fn() }));
vi.mock("@/lib/api-page", () => ({ readPageView: mock.read }));
vi.mock("react", async original => ({ ...await original<typeof import("react")>(), useActionState: () => [{ ok: false }, () => {}, false] }));
vi.mock("next/navigation", () => ({ useRouter: () => ({ push: vi.fn(), refresh: vi.fn() }) }));
import NewStream from "@/app/(app)/stream/new/page";
import EditStream from "@/app/(app)/stream/[id]/edit/page";
import { StreamForm } from "./stream-form";
import { StreamList } from "./stream-list";
const stream = { id: "s", listen: 5354, protocol: "udp" as const, enabled: true, proxyPass: "backend:53" };
const reserved = { process: [3000, 3001, 5432], http: [80, 8080], streamTcp: [], processUdp: [5354] };
const listenHint = (html: string) => /id="stream-listen-hint">([^<]*)<\/div>/.exec(html)?.[1] ?? "";
const form = { action: { id: "stream.createStream", args: [] }, cancelHref: "/stream" };
beforeEach(() => mock.read.mockReset());
it.each([NewStream, () => EditStream({ params: Promise.resolve({ id: "s" }) })])("local stream pages forward a separate UDP reservation list and default missing to empty", async page => {
  for (const processUdp of [[5354], undefined]) {
    mock.read.mockResolvedValue({ stream, reservedPorts: { ...reserved, processUdp } });
    const tree = await page();
    const element = tree.props.children.find((child: { type?: unknown }) => child?.type === StreamForm);
    expect(element.props.reservedUdpListenPorts).toEqual(processUdp ?? []);
    expect(element.props.reservedListenPorts).toEqual([3000, 3001, 5432, 80, 8080]);
  }
});
it.each(["tcp", "udp"] as const)("shows only %s reservations for the selected protocol", protocol => {
  const html = renderToStaticMarkup(<StreamForm {...form} reservedListenPorts={[3000, 3001, 5432, 8080]} reservedUdpListenPorts={[5354]} defaultValues={{ ...stream, listen: 15555, protocol }} />);
  const hint = listenHint(html);
  if (protocol === "udp") { expect(hint).toContain("5354"); expect(hint).not.toContain("5432"); }
  else { expect(hint).toContain("5432"); expect(hint).not.toContain("5354"); }
});
it("remote forms do not invent gateway reservation numbers", () => {
  const html = renderToStaticMarkup(<StreamForm {...form} defaultValues={{ ...stream, listen: 15555 }} />);
  expect(listenHint(html)).toContain("대상 서버"); expect(listenHint(html)).not.toContain("5354");
});
it("saved UDP conflicts are surfaced while TCP-only process ports stay usable for UDP", () => {
  const html = renderToStaticMarkup(<StreamList basePath="/stream" reservedPorts={reserved} data={{ items: [stream, { ...stream, id: "safe", listen: 5432 }], canWrite: false }} />);
  expect(html).toContain("1건의 stream"); expect(html).toContain("포트 5354"); expect(html).not.toContain("포트 5432");
});
