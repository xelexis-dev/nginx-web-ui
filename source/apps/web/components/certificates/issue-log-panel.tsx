import { CodePane, EmptyState, Surface } from "@repo/ui";
import type { IssueLogLineVM } from "./certificate-vm";

/**
 * ACME 발급/갱신 진행 로그(HTTP-01·DNS-01 공통). issuing/검증 중에는 상위
 * `IssuancePoller` 가 2초 간격으로 재조회하므로 이 컴포넌트는 순수 렌더만 한다.
 *
 * **로컬 상세와 노드 상세가 같은 것을 렌더한다** — `(app)/certificates/` 에서
 * 이리로 옮겨왔다(계획 4b Task 6). 소비자가 상세 화면뿐이라 **CSS 모듈도 함께
 * 옮겼다**(복사가 아니라 이동 — 라우트 그룹에 남는 소비자가 0이다).
 *
 * **시각 포맷팅이 여기서 빠져 `toIssueLogLines` 로 갔다.** `at.replace(…)` 가
 * 렌더 도중에 불리면 어긋난 로그가 §9 카드가 아니라 일반 오류 화면을 만든다 —
 * 그 호출은 관문(`toViewModel`) 안에 있어야 한다(`certificate-vm.ts` 헤더).
 */
export function IssueLogPanel({
  entries,
  live,
}: {
  entries: IssueLogLineVM[];
  live: boolean;
}) {
  const title = `발급 진행 로그${live ? " (실시간)" : ""}`;
  return entries.length ? (
    <CodePane
      title={title}
      code={entries
        .map((entry) => `${entry.at} [${entry.level}] ${entry.msg}`)
        .join("\n")}
      copyLabel={false}
    />
  ) : (
    <Surface title={title}>
      <EmptyState description="아직 로그가 없습니다." />
    </Surface>
  );
}
