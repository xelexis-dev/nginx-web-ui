"use client";

import { catchError, type ErrorInfo } from "next/error";
import { unstable_isUnrecognizedActionError } from "next/navigation";
import { ErrorView } from "./error-view";

/**
 * 화면 영역용 오류 경계.
 *
 * 왜 `error.tsx` 가 아니라 `catchError` 인가: 이 앱은 `redirect()` 와 `notFound()` 를
 * 곳곳에서 쓴다(로그인 가드, 액션 후 이동, 없는 항목). 기존 error 경계는 그 둘을
 * 가로채서 오류로 잡아버렸고, 그래서 이 저장소에는 오류 경계가 **하나도 없었다** —
 * 렌더가 실패하면 Next 기본 화면이 그대로 나왔다. Next 16.3 의 `catchError` 는
 * 그 간섭이 없어서 이제 붙일 수 있다.
 *
 * 경계를 `<main>` 안에 두는 것도 의도다. 사이드바와 상단바는 레이아웃이 이미 그린
 * 뒤라 그대로 남고, 실패한 본문만 이 화면으로 바뀐다 — 사용자는 갇히지 않고 다른
 * 메뉴로 그냥 이동할 수 있다.
 *
 * **이 경계가 닿는 범위는 여기까지다.** React 경계는 자기를 감싼 레이아웃의 오류를
 * 잡지 못한다. 셸 레이아웃이 실패하는 경우(DB 장애)는 셸이 직접 try/catch 로 받는다 —
 * 경계를 루트 레이아웃에 하나 더 둬봤지만 서버 렌더 실패에는 뜨지 않았다(실측).
 * 자세한 근거는 `app/error.tsx` 주석에 있다.
 */
function Fallback(
  { homeHref, homeLabel, standalone }: { homeHref: string; homeLabel?: string; standalone?: boolean },
  { error, retry }: ErrorInfo,
) {
  // 배포가 교체된 뒤 오래된 탭이 Server Action 을 부르면 Next 가 전용 오류를
  // 던진다. 그때는 「다시 시도」가 소용없고(같은 오래된 액션을 다시 부른다)
  // 새로고침만이 답이라, 화면을 아예 다르게 낸다. 다른 오류까지 이 문구로
  // 덮으면 진짜 오류를 오인하게 되므로 Next 의 판정 함수로만 가른다.
  const staleDeployment = unstable_isUnrecognizedActionError(error);

  return (
    <ErrorView
      onRetry={retry}
      digest={(error as Error & { digest?: string }).digest}
      homeHref={homeHref}
      homeLabel={homeLabel}
      standalone={standalone}
      staleDeployment={staleDeployment}
    />
  );
}

export default catchError(Fallback);
