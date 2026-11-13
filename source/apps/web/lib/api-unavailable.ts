import { FALLBACK_THEME_CSS } from "./fallback-theme";

/**
 * The screen a person gets while the management API is restarting or unreachable.
 *
 * It has to be a self-contained document: the middleware answers before any React tree exists,
 * and the login/setup screens need the same page when their own API read fails. So this is plain
 * HTML with inline styles — no bundle, no fonts, nothing else to fetch from a server that is
 * already in trouble. It states what happened, retries by itself, and offers the retry the old
 * plain-text answer promised but never gave.
 */
export const RETRY_SECONDS = 5;

export const API_UNAVAILABLE_CSS = `${FALLBACK_THEME_CSS}
  * { box-sizing: border-box; }
  body {
    margin: 0; min-height: 100dvh; display: flex; align-items: center; justify-content: center;
    padding: var(--sp-6) var(--page-x); background: var(--bg-canvas); color: var(--text-1);
    font: var(--fs-md)/var(--lh-md) var(--font);
  }
  .nwui-unavailable {
    display: grid; gap: var(--sp-4); width: 100%; max-width: var(--size-content);
    padding-top: var(--pad-section-top); border-top: var(--bw) solid var(--line-strong);
  }
  .nwui-unavailable .brand { margin: 0; font-size: var(--fs-xs); line-height: var(--lh-xs); font-weight: var(--fw-medium); color: var(--text-3); }
  .nwui-unavailable .status { margin: 0; display: flex; align-items: center; gap: var(--gap-row-icon); font-size: var(--fs-2xl); line-height: var(--lh-2xl); font-weight: var(--fw-semibold); color: var(--danger-text); }
  .nwui-unavailable .status::before { content: ""; width: var(--size-dot); height: var(--size-dot); flex: none; background: var(--danger); }
  .nwui-unavailable .rule { display: none; }
  .nwui-unavailable h1 { margin: 0; font-size: var(--fs-2xl); font-weight: var(--fw-semibold); line-height: var(--lh-2xl); letter-spacing: var(--ls-tight); overflow-wrap: anywhere; }
  .nwui-unavailable p { margin: 0; color: var(--text-2); font-size: var(--fs-md); line-height: var(--lh-md); }
  .nwui-unavailable .actions { display: flex; flex-wrap: wrap; gap: var(--gap-actions); }
  .nwui-unavailable button, .nwui-unavailable a.link {
    display: inline-flex; align-items: center; justify-content: center;
    appearance: none; border: var(--bw) solid var(--border-control); border-radius: var(--r-2);
    min-height: var(--size-ctl); padding: 0 var(--ctl-x); font: var(--fs-md)/var(--lh-md) var(--font); font-weight: var(--fw-medium); cursor: pointer; text-decoration: none;
  }
  .nwui-unavailable button { background: var(--accent); color: var(--on-accent); border-color: transparent; }
  .nwui-unavailable a.link { background: var(--bg-control); color: var(--text-1); }
  .nwui-unavailable :is(button,a):focus-visible { outline: var(--bw-2) solid var(--focus); outline-offset: var(--sp-0-5); }
  .nwui-unavailable button:hover { background: var(--accent-hover); }
  .nwui-unavailable button:active { background: var(--accent-press); }
  @media (max-width: 768px) { .nwui-unavailable h1 { font-size: var(--fs-xl); line-height: var(--lh-xl); } }
  .nwui-unavailable .countdown { font-size: var(--fs-xs); line-height: var(--lh-xs); font-variant-numeric: tabular-nums; }`;

export const API_UNAVAILABLE_TITLE = "API 서버에 연결하지 못했습니다";
export const API_UNAVAILABLE_BODY = "관리 화면이 API 서버의 응답을 받지 못했습니다. 컨테이너가 다시 시작하는 중이면 잠시 뒤 저절로 열립니다.";
export const API_UNAVAILABLE_NOTE = "저장된 설정과 실행 중인 nginx 는 이 화면과 무관하게 그대로 동작합니다.";

/** Auto-retry for the plain document; the React screen uses the same countdown. */
const RELOAD_SCRIPT = `(function(){var left=${RETRY_SECONDS};var label=document.getElementById("nwui-retry-left");setInterval(function(){left-=1;if(label)label.textContent=String(left>0?left:0);if(left<=0)location.reload();},1000);})();`;

export function apiUnavailableHtml(): string {
  return `<!DOCTYPE html>
<html lang="ko">
<head>
<meta charSet="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>연결할 수 없음 · nginx-web-ui</title>
<style>${API_UNAVAILABLE_CSS}</style>
</head>
<body>
  <div class="nwui-unavailable" role="alert">
    <p class="brand">nginx-web-ui</p>
    <p class="status">503 Service Unavailable</p>
    <div class="rule"></div>
    <h1>${API_UNAVAILABLE_TITLE}</h1>
    <p>${API_UNAVAILABLE_BODY}</p>
    <p>${API_UNAVAILABLE_NOTE}</p>
    <div class="actions">
      <button type="button" onclick="location.reload()">다시 시도</button>
      <a class="link" href="/">처음으로</a>
    </div>
    <p class="countdown"><span id="nwui-retry-left">${RETRY_SECONDS}</span>초 뒤 자동으로 다시 시도합니다.</p>
  </div>
  <script>${RELOAD_SCRIPT}</script>
</body>
</html>`;
}

export function apiUnavailableResponse(): Response {
  return new Response(apiUnavailableHtml(), {
    status: 503,
    headers: {
      "Content-Type": "text/html; charset=utf-8",
      "Cache-Control": "no-store",
      "Retry-After": String(RETRY_SECONDS),
    },
  });
}