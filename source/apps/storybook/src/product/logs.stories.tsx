import { useEffect, useState } from "react";
import { PageHeader } from "@repo/ui";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, waitFor, within } from "storybook/test";
import { LogViewer } from "@/components/logs/log-viewer";
import layout from "@/components/page-layout.module.css";
import { NodeErrorCard } from "@/components/node-error";
import { ProductFrame } from "./frame";

const message =
  '42#42: *7 upstream timed out while reading response header from upstream, client: 192.0.2.10, server: gateway.example.test, request: "GET /verylongrequestpathwithoutwordbreakstochecktheexpandederrormessage HTTP/1.1", upstream: "http://upstream.example.test:8080/api/report", host: "gateway.example.test"';
const samples = {
  // The second line does not parse: it is shown as one full-width row, which must not narrow the path column.
  access: [
    '192.0.2.10 - - [28/Sep/2026:10:00:00 +0000] "GET /api/report HTTP/1.1" 200 512 "-" "Example browser" "gateway.example.test"',
    "upstream sent an unparsable access line",
  ],
  error: [
    `2026/09/28 10:00:00 [error] ${message}`,
    "2026/09/28 10:01:00 [warn] 42#42: upstream response is buffered to a temporary file",
  ],
};

/** The real viewer with an in-memory EventSource; no API request or live log data. */
function LogsPreview({ nodeName, offline = false, longTable = false }: { nodeName?: string; offline?: boolean; longTable?: boolean }) {
  const [ready, setReady] = useState(false);
  useEffect(() => {
    const original = window.EventSource;
    class PreviewStream {
      onopen: ((event: Event) => void) | null = null;
      onmessage: ((event: MessageEvent) => void) | null = null;
      onerror: ((event: Event) => void) | null = null;
      private timer: ReturnType<typeof setTimeout>;
      constructor(url: string | URL) {
        const file =
          new URL(String(url), location.origin).searchParams.get("file") ===
          "error"
            ? "error"
            : "access";
        this.timer = setTimeout(() => {
          this.onopen?.(new Event("open"));
          const rows = longTable ? Array.from({ length: 40 }, (_, i) => file === "access"
            ? samples.access[0]!.replace("/api/report", `/api/report/${i}/a-long-request-path-without-shortening`).replace("Example browser", "Long browser user agent ".repeat(24))
            : samples.error[i % samples.error.length]!) : samples[file];
          rows.forEach((data) =>
            this.onmessage?.(new MessageEvent("message", { data })),
          );
        }, 0);
      }
      close() {
        clearTimeout(this.timer);
      }
    }
    window.EventSource = PreviewStream as unknown as typeof EventSource;
    setReady(true);
    return () => {
      window.EventSource = original;
    };
  }, [longTable]);
  return (
    <section className={layout.page}>
      <PageHeader

        title="로그"
        info="access.log / error.log를 실시간으로 확인합니다."
      />
      <div>{offline ? <NodeErrorCard nodeId="preview" failure={{ kind: "unreachable", message: "노드에 연결할 수 없습니다." }} retryHref="/nodes/preview/logs" listHref="/nodes/preview/logs" /> : ready ? <LogViewer streamPath="/storybook/logs" /> : null}</div>
    </section>
  );
}

const meta = {
  id: "product-logs",
  title: "Product/Logs",
  component: LogsPreview,
  tags: ["!autodocs"],
  parameters: {
    layout: "fullscreen",
    docs: {
      description: {
        component:
          "실제 LogViewer에 메모리 로그를 전달합니다. 행 클릭 또는 Enter·Space로 상세 내용을 펼칩니다. 네트워크 요청은 보내지 않습니다.",
      },
    },
  },
  decorators: [(Story, context) => <ProductFrame path={context.args.nodeName ? "/nodes/preview/logs" : "/logs"} nodeName={context.args.nodeName as string | undefined} available={!context.args.offline} label="로그" role="editor" storyId={context.id} viewport={undefined}><Story /></ProductFrame>],
} satisfies Meta<typeof LogsPreview>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Access: Story = { play: async ({ canvasElement }) => {
  const canvas = within(canvasElement);
  await expect(canvas.findByText("연결됨", { exact: true })).resolves.toBeVisible();
  await expect(canvasElement.querySelectorAll("[data-pulse]")).toHaveLength(1);
} };
export const ErrorDetails: Story = {
  name: "error.log · 긴 메시지 상세",
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(
      await canvas.findByRole("button", { name: "error.log" }),
    );
    const row = (await canvas.findByText(message)).closest("tr")!;
    await userEvent.click(row);
    await waitFor(() => expect(canvas.getByText("전체 메시지")).toBeVisible());
    expect(row).toHaveAttribute("aria-expanded", "true");
  },
};
export const ErrorDetailsMobile: Story = {
  ...ErrorDetails,
  name: "error.log · 390",
  globals: { viewport: { value: "p390", isRotated: false } },
};

export const NodeOnline: Story = { name: "노드 · 응답함", args: { nodeName: "서울 운영 노드의 아주 긴 이름을 확인하는 미리보기" } };
export const NodeOffline: Story = { name: "노드 · 응답 없음", args: { nodeName: "서울 운영 노드의 아주 긴 이름을 확인하는 미리보기", offline: true } };

export const Raw: Story = {
  name: "Raw · 원문 보존",
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(await canvas.findByRole("button", { name: "Raw" }));
    await expect(canvas.findByText(samples.access[0]!, { exact: true })).resolves.toBeVisible();
    await expect(canvasElement.querySelector('[data-log-view="raw"]')).not.toBeNull();
  },
};

export const WideTable: Story = { name: "긴 UA와 스크롤 헤더", args: { longTable: true } };
export const NodeWideTable: Story = { name: "노드 · 긴 UA와 스크롤 헤더", args: { nodeName: "미리보기 노드", longTable: true } };
export const ErrorWideTable: Story = { name: "error.log · 스크롤 헤더", args: { longTable: true }, play: async ({ canvasElement }) => { await userEvent.click(await within(canvasElement).findByRole("button", { name: "error.log" })); } };
