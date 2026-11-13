"use client";

// Font only: if this chunk does not arrive, the stack below falls back to system fonts and the
// screen still works (every other style stays inline, see below).
import "@repo/ui/fonts.css";
import { FALLBACK_THEME_CSS } from "@/lib/fallback-theme";

/**
 * 최후의 화면 — 루트 레이아웃/템플릿이 **클라이언트에서** 실패했을 때를 위한 것이다.
 *
 * ⚠ 서버 렌더 실패에는 뜨지 않는다. 루트 레이아웃이 서버에서 던지게 만들어 확인해보니
 * Next 의 기본 오류 문서(`<html id="__next_error__">`)가 나갔고, 프리렌더 산출물
 * `_global-error.html` 에도 이 컴포넌트의 내용이 들어 있지 않았다. 문서상 이 파일이
 * 담당하는 경우이지만 실제로는 그렇지 않으니, 이걸 믿고 다른 방어를 빼지 말 것.
 *
 * 이 화면은 루트 레이아웃을 **대체**하므로 `<html>`·`<body>` 를 직접 그려야 하고,
 * 루트 레이아웃이 불러오던 `tokens.css`·`globals.css`·상태 화면 CSS 가 하나도 없다.
 * 그래서 스타일을 파일 안에 박아 자립시킨다 — 여기서 CSS 청크에 기대면 "레이아웃이
 * 깨진 오류 화면"이라는 최악의 조합이 나온다.
 *
 * 같은 이유로 테마도 스스로 판단한다. 루트 레이아웃의 테마 스크립트가 돌지 않았으니
 * `data-theme` 이 없고, `prefers-color-scheme` 만 믿을 수 있다(사용자가 앱에서 고른
 * 테마와 어긋날 수 있지만, 이 화면이 뜨는 상황에서는 그게 최선이다).
 *
 * 본문 오류 실패는 여기까지 오지 않는다 — 셸 안쪽은 `components/error-boundary.tsx`
 * 가 먼저 잡아서 사이드바를 남긴 채 보여준다.
 */
export default function GlobalError({
  error,
  retry,
}: {
  error: Error & { digest?: string };
  retry: () => void;
}) {
  return (
    <html lang="ko">
      <body>
        <style>{CSS}</style>
        <div className="wrap">
          <div className="card">
            <div className="protocol">HTTP/1.1</div>
            <p className="status">500 Internal Server Error</p>
            <div className="rule" />
            <h1 className="title">화면을 불러오지 못했습니다</h1>
            <p className="desc">
              다시 시도하면 대부분 해결됩니다. 계속 같은 화면이 뜨면 아래 오류 ID로 서버 로그를
              확인하세요.
            </p>
            {error.digest ? (
              <p className="digest">
                <span className="digestLabel">오류 ID</span>
                <span className="digestValue">{error.digest}</span>
              </p>
            ) : null}
            <div className="actions">
              <button type="button" className="action primary" onClick={() => retry()}>
                다시 시도
              </button>
              <a className="action" href="/">
                처음으로
              </a>
            </div>
          </div>
        </div>
      </body>
    </html>
  );
}

/* 값은 packages/ui/src/tokens.css 의 토큰과 같다. 토큰 파일을 못 읽는 화면이라
   그대로 옮겨 적었다 — 토큰이 바뀌면 여기도 함께 고칠 것. */
const CSS = `${FALLBACK_THEME_CSS}
* { box-sizing: border-box; }
body { margin: 0; background: var(--bg-canvas); color: var(--text-1); font-family: var(--font); font-size: var(--fs-md); line-height: var(--lh-md); }
.wrap { display: flex; align-items: center; justify-content: center; min-height: 100dvh; padding: var(--sp-6) var(--page-x); }
.card { display: grid; gap: var(--sp-4); width: 100%; max-width: var(--size-content); padding-top: var(--pad-section-top); border-top: var(--bw) solid var(--line-strong); }
.protocol { font-size: var(--fs-xs); line-height: var(--lh-xs); font-weight: var(--fw-medium); color: var(--text-3); }
.status { display: flex; align-items: center; gap: var(--gap-row-icon); margin: 0; font-size: var(--fs-2xl); line-height: var(--lh-2xl); font-weight: var(--fw-semibold); letter-spacing: var(--ls-tight); font-variant-numeric: tabular-nums; color: var(--danger-text); }
.status::before { content: ""; width: var(--size-dot); height: var(--size-dot); flex: none; background: var(--danger); }
.rule { display: none; }
.title { margin: 0; font-size: var(--fs-2xl); line-height: var(--lh-2xl); font-weight: var(--fw-semibold); letter-spacing: var(--ls-tight); overflow-wrap: anywhere; }
.desc { margin: 0; font-size: var(--fs-md); line-height: var(--lh-md); color: var(--text-2); }
.digest { display: flex; align-items: baseline; gap: var(--sp-2); flex-wrap: wrap; margin: 0; padding: var(--sp-3) var(--sp-4); background: var(--bg-sunken); border-radius: var(--r-2); }
.digestLabel { font-size: var(--fs-xs); line-height: var(--lh-xs); color: var(--text-2); }
.digestValue { font-size: var(--fs-xs); line-height: var(--lh-xs); font-variant-numeric: tabular-nums; overflow-wrap: anywhere; }
.actions { display: flex; gap: var(--gap-actions); flex-wrap: wrap; }
.action {
  display: inline-flex; align-items: center; justify-content: center; min-height: var(--size-ctl); padding: 0 var(--ctl-x);
  border: var(--bw) solid var(--border-control); border-radius: var(--r-2); background: var(--bg-control); color: var(--text-1);
  font-family: inherit; font-size: var(--fs-md); line-height: var(--lh-md); font-weight: var(--fw-medium);
  text-decoration: none; white-space: nowrap; cursor: pointer;
}
.action:hover { background: var(--bg-hover); border-color: var(--border-control-hover); }
.action:focus-visible { outline: var(--bw-2) solid var(--focus); outline-offset: var(--sp-0-5); }
.primary { background: var(--accent); border-color: var(--accent); color: var(--on-accent); }
.primary:hover { background: var(--accent-hover); border-color: var(--accent-hover); }
.primary:active { background: var(--accent-press); }
@media (max-width: 768px) { .title { font-size: var(--fs-xl); line-height: var(--lh-xl); } }
`;
