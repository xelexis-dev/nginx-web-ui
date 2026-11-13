"use client";

import { unstable_isUnrecognizedActionError } from "next/navigation";
import { ErrorView } from "@/components/error-view";

/**
 * 루트 세그먼트의 오류 경계 — **클라이언트에서 난 오류**를 받는다.
 *
 * ⚠ 서버 렌더가 실패하는 경우는 여기까지 오지 않는다. 실측으로 확인한 것:
 * 셸 레이아웃이나 루트 세그먼트 페이지가 **서버 렌더 중** 던지면, 아직 아무것도
 * flush 되지 않아 React 가 fallback 을 흘려보낼 자리가 없다 — Next 는 기본 오류
 * 문서(`<html id="__next_error__">`)를 그대로 보낸다. `app/error.tsx` 도,
 * 루트 레이아웃 `<body>` 안의 `catchError` 경계도, `global-error` 도 뜨지 않았다.
 *
 * (루트 레이아웃에 `<Suspense>` 를 넣으면 셸이 먼저 flush 돼서 이 화면이 뜬다.
 * 실제로 뜨는 것까지 확인했지만 채택하지 않았다 — 라우트 프리렌더 방식이 바뀌어
 * 빌드가 깨졌고, 첫 flush 가 빈 셸이라 하드 내비게이션에서 흰 화면이 스친다.
 * 그건 Cache Components 전환과 함께 다룰 일이다.)
 *
 * 그래서 현실적인 실패(**DB 장애로 세션 확인이 깨지는 경우**)는 경계에 맡기지 않고
 * 셸 레이아웃이 직접 try/catch 로 받는다 — `(app)/layout.tsx` 참고.
 * 이 파일은 하이드레이션 이후 클라이언트에서 터지는 오류를 위한 마지막 그물이다.
 */
export default function ShellError({
  error,
  retry,
}: {
  error: Error & { digest?: string };
  retry: () => void;
}) {
  // 배포 교체 뒤 오래된 탭이 Server Action 을 부르면 Next 가 이 오류를 던진다.
  // 다른 오류까지 "새로고침하세요"로 덮으면 진짜 오류를 오인하게 되므로,
  // Next 가 내놓은 판정 함수로만 가른다.
  const stale = unstable_isUnrecognizedActionError(error);

  return (
    <ErrorView
      onRetry={retry}
      digest={error.digest}
      homeHref="/"
      standalone
      staleDeployment={stale}
    />
  );
}
