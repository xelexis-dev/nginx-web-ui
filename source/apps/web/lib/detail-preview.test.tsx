import { beforeEach, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
const mocks = vi.hoisted(() => ({ read: vi.fn() }));
vi.mock("@/lib/api-page", () => ({ readPageView: mocks.read }));
vi.mock("@repo/presentation/nginx", () => { throw new Error("nginx runtime must stay in API"); });
vi.mock("@repo/db", () => { throw new Error("DB runtime must stay in API"); });
import ServerPage from "@/app/(app)/reverse-proxy/[id]/page";
import StreamPage from "@/app/(app)/stream/[id]/page";
import TemplatesPage from "@/app/(app)/reverse-proxy/templates/page";
import { ServerDetail } from "@/components/reverse-proxy/server-detail";
import { StreamDetail } from "@/components/streams/stream-detail";
import { configPreview } from "./config-preview";
beforeEach(() => vi.clearAllMocks());
it("template gallery reads API previews and respects API write permission",async()=>{
  const previews=Object.fromEntries(["nexus","nexus-docker","nexus-combined","gitlab","gitlab-registry"].map(id=>[id,{config:`# api template ${id}`,error:null}]));
  mocks.read.mockResolvedValue({canWrite:false,previews});const html=renderToStaticMarkup(await TemplatesPage());
  expect(mocks.read).toHaveBeenLastCalledWith("/reverse-proxy/templates");expect(html).toContain("# api template nexus-combined");expect(html).toContain("GitLab Container Registry");expect(html).not.toContain("이 템플릿으로 생성");
});
it("local detail pages display API-rendered previews including target certificate paths", async () => {
  mocks.read.mockResolvedValueOnce({server:{id:"s",serverName:"example.test",listen:80,enabled:true,config:{},upstreamId:null,certificateId:null},upstreams:[],certificates:[],canWrite:false,selfProxyNotice:false,preview:{config:"ssl_certificate /api-instance/certs/c/fullchain.pem;",error:null}});
  const server=renderToStaticMarkup(await ServerPage({params:Promise.resolve({id:"s"})}));
  expect(server).toContain("/api-instance/certs/c/fullchain.pem");expect(mocks.read).toHaveBeenLastCalledWith("/reverse-proxy/s");
  mocks.read.mockResolvedValueOnce({stream:{id:"l",listen:9000,protocol:"tcp",proxyPass:"127.0.0.1:99",enabled:false},canWrite:false,preview:{config:"# rendered by Rust\nserver { listen 9000; }",error:null}});
  const stream=renderToStaticMarkup(await StreamPage({params:Promise.resolve({id:"l"})}));
  expect(stream).toContain("# rendered by Rust");expect(stream).toContain("비활성 상태");expect(mocks.read).toHaveBeenLastCalledWith("/stream/l");
});
it("remote detail uses the remote preview and old nodes get an explicit unavailable message", () => {
  const html=renderToStaticMarkup(<ServerDetail basePath="/nodes/n/reverse-proxy" configOrigin="remote-node" write={null} data={{id:"s",serverName:"example.test",listen:80,enabled:true,config:{},upstreamId:null,certificateId:null,upstreams:[],cert:null,selfProxyNotice:false,preview:{config:"ssl_certificate /remote/cert.pem;",error:null}}}/>);
  expect(html).toContain("/remote/cert.pem");expect(html).toContain("노드의 현재 값과 인증서 경로");expect(html).not.toContain("게이트웨이의 설정");
  const unavailable=renderToStaticMarkup(<StreamDetail basePath="/nodes/n/stream" canWrite={false} toggleAction={{id:"node.toggleNodeStream",args:["n"]}} deleteAction={{id:"node.deleteNodeStream",args:["n"]}} data={{id:"l",listen:9000,protocol:"tcp",proxyPass:"127.0.0.1:99",enabled:true}}/>);
  expect(unavailable).toContain("이 노드는 설정 미리보기를 제공하지 않습니다");
  expect(() => configPreview({config:42,error:null})).toThrow("Invalid configuration preview");
});

const hint = "이름 확인 없이 만든 미리보기입니다 — 실제 적용에서는 이 경로의 결과가 달라질 수 있습니다";
const paths = ["/api", "/long-" + "x".repeat(300)];
it("passes optional unresolved paths and drops malformed advisory data without mutating input", () => {
  const base = { config: "# preview", error: null };
  expect(configPreview(base)).toEqual(base);
  expect(configPreview({ ...base, unresolvedPaths: paths })).toEqual({ ...base, unresolvedPaths: paths });
  expect(configPreview({ ...base, unresolvedPaths: [] })).toEqual({ ...base, unresolvedPaths: [] });
  for (const value of [null, "bad", {}, [""], ["/ok", 1], [false]]) {
    const input = { ...base, unresolvedPaths: value };
    expect(configPreview(input)).toEqual(base);
    expect(input.unresolvedPaths).toBe(value);
    expect(configPreview({ config: null, error: "preview failed", unresolvedPaths: value })).toEqual({ config: null, error: "preview failed" });
  }
  expect(() => configPreview({ config: 42, error: null, unresolvedPaths: paths })).toThrow("Invalid configuration preview");
});
it.each(["server", "templates"])("shows every unresolved path beside %s preview, absent and empty stay quiet", async (kind) => {
  for (const unresolvedPaths of [paths, undefined, []]) {
    const preview = { config: "# authoritative API preview", error: null, unresolvedPaths };
    let html: string;
    if (kind === "server") html = renderToStaticMarkup(<ServerDetail basePath="/reverse-proxy" configOrigin="this-instance" write={null} data={{id:"s",serverName:"example.test",listen:80,enabled:true,config:{},upstreamId:null,certificateId:null,upstreams:[],cert:null,selfProxyNotice:false,preview}} />);
    else { mocks.read.mockResolvedValue({ canWrite: false, previews: { nexus: preview } }); html = renderToStaticMarkup(await TemplatesPage()); }
    expect(html).toContain("# authoritative API preview");
    if (unresolvedPaths?.length) {
      expect(html).toContain(hint);
      for (const path of paths) expect(html).toContain(path);
      expect(html).toMatch(/<ul\b[^>]*>[\s\S]*<li\b[^>]*><code>\/api<\/code>/);
      expect(html).not.toContain('role="listitem"');
      expect(html).not.toMatch(/<p\b[^>]*>(?:(?!<\/p>).)*<ul\b/s);
    }
    else expect(html).not.toContain(hint);
  }
});

it("stream uses only the whole-stream sentence, without rendering advisory labels as paths", () => {
  const text = "이름 확인 없이 만든 미리보기입니다 — 실제 적용에서는 결과가 달라질 수 있습니다";
  for (const unresolvedPaths of [["stream"], undefined, []]) {
    const html = renderToStaticMarkup(<StreamDetail basePath="/stream" canWrite={false} toggleAction={{id:"stream.toggleStream",args:[]}} deleteAction={{id:"stream.deleteStream",args:[]}} data={{id:"s",listen:9000,protocol:"tcp",proxyPass:"app.test:99",enabled:true,preview:{config:"# stream preview",error:null,unresolvedPaths}}} />);
    expect(html).toContain("# stream preview");
    expect(html).not.toContain('role="listitem"');
    if (unresolvedPaths?.length) expect(html).toContain(text);
    else expect(html).not.toContain(text);
  }
});
