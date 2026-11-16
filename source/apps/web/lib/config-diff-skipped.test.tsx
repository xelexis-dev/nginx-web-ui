import { expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { readConfigDiff } from "./config-diff";
import type { ConfigDiff } from "./config-diff";
import { ConfigDiffView } from "@/components/config-diff-view";

function diff(over: Partial<ConfigDiff>): ConfigDiff {
  return { servers: [], streams: [], upstreams: [], hasChanges: true, ...over };
}

// 적용이 거부한 행은 "아직 안 올라간 새 행"과 같은 모양으로 보이면 안 된다 — 기다리는 게 아니라
// 빠진 것이고, 왜 빠졌는지가 화면에 있어야 사용자가 고칠 수 있다.
it("적용에서 제외된 행을 이유와 함께 그린다", () => {
  const html = renderToStaticMarkup(
    <ConfigDiffView
      diff={diff({
        servers: [
          {
            key: "bad",
            label: "broken.test:80",
            status: "skipped",
            changes: [],
            reason: "listen 443 이 이미 쓰이고 있습니다",
          },
          { key: "new", label: "fresh.test:80", status: "added", changes: [] },
        ],
      })}
    />,
  );
  expect(html).toContain("적용에서 제외됨");
  expect(html).toContain("listen 443 이 이미 쓰이고 있습니다");
  // 새로 추가된 행에는 붙지 않는다.
  expect(html.match(/적용에서 제외됨/g)).toHaveLength(2);
});

it("이유가 없는 제외 행도 그린다", () => {
  const html = renderToStaticMarkup(
    <ConfigDiffView
      diff={diff({
        streams: [
          { key: "s1", label: "15433/tcp", status: "skipped", changes: [] },
        ],
      })}
    />,
  );
  expect(html).toContain("적용에서 제외됨");
  expect(html).toContain("15433/tcp");
});

// 원격 노드 응답을 읽는 쪽도 새 상태를 알아야 한다 — 모르면 화면 전체가 TypeError 로 죽는다.
it("원격 응답의 skipped 상태와 이유를 그대로 나른다", () => {
  const read = readConfigDiff(
    diff({
      upstreams: [
        {
          key: "u1",
          label: "api",
          status: "skipped",
          changes: [],
          reason: "대상 서버가 없습니다",
        },
      ],
    }),
  );
  expect(read.upstreams[0]).toMatchObject({
    status: "skipped",
    reason: "대상 서버가 없습니다",
  });
});
