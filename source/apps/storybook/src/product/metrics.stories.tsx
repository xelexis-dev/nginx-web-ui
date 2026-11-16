import { useEffect, useState } from "react";
import { PageHeader } from "@repo/ui";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, waitFor, within } from "storybook/test";
import { MetricsView } from "@/components/metrics/metrics-view";
import { RANGES } from "@/lib/metrics-view";
import layout from "@/components/page-layout.module.css";
import { NodeErrorCard } from "@/components/node-error";
import { ProductFrame, productViewports } from "./frame";

const fixturePath = "/storybook/metrics-fixture";
/** The fixture's last bucket. The health line is judged against END + sampleAgeMs, so every run reads the same. */
const END = Date.UTC(2026, 9, 2, 5, 0);
const DOMAIN_HOSTS = ["api.example.test", "files.example.test", "auth.example.test", "static.example.test", "admin-console.example.test", "__other__"];
/** Six lines that end close together, so the line patterns (not only colour) have to tell them apart. */
const DOMAIN_SCALE = [1, 0.5, 0.42, 0.38, 0.3, 0.25];
/** Intercept only this story's URL. No API request, dependency or product data path changes. */
function MetricsPreview({ empty = false, domains = 2, sampleAgeMs = 30_000, longHosts = false, nodeName, offline = false }: { nodeName?: string; offline?: boolean; longHosts?: boolean; empty?: boolean; domains?: 2 | 6; sampleAgeMs?: number }) {
  const [ready, setReady] = useState(false);
  useEffect(() => {
    const original = window.fetch;
    const hosts = longHosts ? DOMAIN_HOSTS.map((host, i) => i === 0 ? "long-domain-name-for-table-ellipsis.preview-metrics.example.test" : host) : DOMAIN_HOSTS;
    const fixtureFetch: typeof fetch = async (input, init) => {
      const url = new URL(input instanceof Request ? input.url : String(input), location.origin);
      if (url.pathname !== fixturePath) return original(input, init);
      if (init?.signal?.aborted) throw new DOMException("Aborted", "AbortError");
      const range = RANGES.find(r => r.key === url.searchParams.get("range")) ?? RANGES[0]!;
      const end = END;
      const values = [42, 54, 49, 68, 75, null, 64, 81, 73, 66, 59, 62];
      const times = empty ? [] : values.map((_, i) => end - (values.length - 1 - i) * range.ms / (values.length - 1));
      return new Response(JSON.stringify({
        range: range.key,
        global: times.map((t, i) => ({ t, active: values[i], requestsPerSec: values[i] === null ? null : values[i]! * 2, connectionsPerSec: values[i] === null ? null : values[i]! / 4, droppedPerSec: values[i] === null ? null : 0 })),
        domains: { times, source: "minutes", otherTotal: 0,
          series: empty ? [] : hosts.slice(0, domains).map((host, d) => ({ host, points: values.map(v => v === null ? null : Math.round(v * DOMAIN_SCALE[d]!)), total: Math.round(724 * DOMAIN_SCALE[d]!) })),
          ranking: empty ? [] : hosts.slice(0, domains).map((host, d) => ({ host, requests: Math.round(724 * DOMAIN_SCALE[d]!), bytesSent: Math.round(741376 * DOMAIN_SCALE[d]!), s4xx: d === 0 ? 3 : 0, s5xx: d === 1 ? 1 : 0 })) },
        health: { lastSampleAt: empty ? null : end, lastBucketAt: empty ? null : end, unparsedCount: 0, parseRatio: empty ? null : 1 },
      }), { status: 200, headers: { "Content-Type": "application/json" } });
    };
    window.fetch = fixtureFetch;
    setReady(true);
    return () => { if (window.fetch === fixtureFetch) window.fetch = original; };
  }, [empty, domains, longHosts]);
  return <section className={layout.page}>
    <PageHeader title="접속 지표" info="nginx 연결·요청 추이와 도메인별 요청 수입니다. 1분마다 수집합니다." />
    {offline ? <NodeErrorCard nodeId="preview" failure={{ kind: "unreachable", message: "노드에 연결할 수 없습니다." }} retryHref="/nodes/preview/metrics" listHref="/nodes/preview/metrics" /> : ready ? <MetricsView initialRange="1h" dataPath={fixturePath} now={() => END + sampleAgeMs} /> : null}
  </section>;
}
const meta = {
  id: "product-metrics", title: "Product/접속 지표", component: MetricsPreview, tags: ["!autodocs"],
  parameters: { layout: "fullscreen",
    viewport: { options: { ...productViewports, p320: { name: "Mobile 320", styles: { width: "320px", height: "640px" }, type: "mobile" } } },
    docs: { description: { component: "실제 MetricsView와 TimeSeriesChart입니다. 고정 예시 표본으로 중간 수집 공백을 포함하며 이 Storybook 전용 URL만 메모리 응답으로 처리합니다. 계열은 색만으로 가리지 않습니다: 이름은 범례·툴팁·표에 글자로 있고, 4–6번째 계열은 선 무늬를 쓰며, 그 표식은 선과 같은 색·같은 무늬의 짧은 선입니다. 차트 오른쪽 끝에는 글자를 두지 않습니다. 수집 상태는 5분이 지나면 문구가 「수집 지연」으로 바뀝니다." } } },
  decorators: [(Story, context) => <ProductFrame path={context.args.nodeName ? "/nodes/preview/metrics" : "/metrics"} nodeName={context.args.nodeName as string | undefined} available={!context.args.offline} label="접속 지표" role="editor" storyId={context.id} viewport={undefined}><Story /></ProductFrame>],
} satisfies Meta<typeof MetricsPreview>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = { play: async ({ canvasElement }) => { const canvas = within(canvasElement);
  await expect(canvas.findByRole("table", { name: "도메인별 요청 수" })).resolves.toBeVisible();
  await expect(canvas.getByRole("heading", { name: "연결과 요청", level: 2 })).toBeVisible();
  for (const name of ["활성 연결", "초당 요청 / 연결", "드롭된 연결", "도메인별 요청 수"]) {
    await expect(canvas.getByRole("heading", { name, level: 3 })).toBeVisible();
  } } };
export const Empty: Story = { name: "수집 전", args: { empty: true } };
export const Narrow: Story = { name: "접속 지표 · 390", globals: { viewport: { value: "p390", isRotated: false } } };
/** 마지막 표본이 10분 전: 점 색만이 아니라 문구로 「수집 지연」을 알린다. */
export const Stale: Story = {
  name: "수집 지연",
  args: { sampleAgeMs: 10 * 60_000 },
  play: async ({ canvasElement }) => { await expect(within(canvasElement).findByText(/^수집 지연 · 마지막 표본/)).resolves.toBeVisible(); },
};
/** 도메인 6개: 4–6번째 계열은 선 무늬로도 구분되고, 범례 표식이 같은 무늬다. 차트 오른쪽 끝에는 글자가 없다. */
export const SixDomains: Story = {
  name: "도메인 6개 · 선 무늬",
  args: { domains: 6 },
  play: async ({ canvasElement }) => {
    const chart = await within(canvasElement).findByRole("img", { name: "도메인별 요청 수 추이" });
    await waitFor(() => expect(chart.querySelectorAll("path[stroke-dasharray]")).toHaveLength(3));
    const table = await within(canvasElement).findByRole("table", { name: "도메인별 요청 수" });
    await expect(within(table).getByText("(기타 요청)")).toHaveAttribute("title", "등록한 서버 이름에 맞지 않는 요청(관리 화면 접속 포함)과 도메인 수 상한을 넘은 요청을 합산합니다.");
    // Only axis ticks remain as text: none starts in the 16px right padding.
    await expect(chart.getBoundingClientRect().height).toBe(192);
    const plotRight = chart.getBoundingClientRect().width - 16;
    for (const text of chart.querySelectorAll("text")) await expect((text as SVGTextElement).getBBox().x).toBeLessThan(plotRight);
    await expect(chart.parentElement!.querySelectorAll("li svg line[stroke-dasharray]")).toHaveLength(3);
  },
};
export const SixDomainsNarrow: Story = { ...SixDomains, name: "도메인 6개 · 390", globals: { viewport: { value: "p390", isRotated: false } } };
/** 320px: 좁은 폭에서도 범례의 선 무늬 표식이 계열을 잇는다. */
export const SixDomains320: Story = { name: "도메인 6개 · 320", args: { domains: 6 }, globals: { viewport: { value: "p320", isRotated: false } } };

/** Long host labels truncate inside the flex row; the full value remains in title. */
export const LongHosts: Story = {
  name: "긴 도메인 이름",
  args: { longHosts: true },
  play: async ({ canvasElement }) => {
    const table = await within(canvasElement).findByRole("table", { name: "도메인별 요청 수" });
    const label = within(table).getByTitle("long-domain-name-for-table-ellipsis.preview-metrics.example.test");
    await waitFor(() => {
      expect(getComputedStyle(label).textOverflow).toBe("ellipsis");
      expect(label.scrollWidth).toBeGreaterThan(label.clientWidth);
      expect(label.getBoundingClientRect().right).toBeLessThanOrEqual(label.closest("td")!.getBoundingClientRect().right);
    });
  },
};

export const NodeOnline: Story = { name: "노드 · 응답함", args: { nodeName: "서울 운영 노드의 아주 긴 이름을 확인하는 미리보기" } };
export const NodeOffline: Story = { name: "노드 · 응답 없음", args: { nodeName: "서울 운영 노드의 아주 긴 이름을 확인하는 미리보기", offline: true } };
