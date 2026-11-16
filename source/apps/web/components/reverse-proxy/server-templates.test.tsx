import { expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { ServerTemplates } from "./server-templates";

it.each([true, false])(
  "keeps list navigation without a templates eyebrow (write=%s)",
  (canWrite) => {
    const html = renderToStaticMarkup(
      <ServerTemplates
        items={[]}
        canWrite={canWrite}
        navigation={
          <nav aria-label="프록시 구성">
            <a href="/reverse-proxy">서버</a>
          </nav>
        }
      />,
    );
    const header = html.match(/<header\b[^>]*>([\s\S]*?)<\/header>/)![1]!;
    expect(header).toContain("서버 템플릿");
    expect(header).not.toContain('href="/reverse-proxy"');
    expect(html).toContain('aria-label="프록시 구성"');
    expect(html).toContain('href="/reverse-proxy"');
    expect(header.includes("빈 폼으로 생성")).toBe(canWrite);
  },
);
