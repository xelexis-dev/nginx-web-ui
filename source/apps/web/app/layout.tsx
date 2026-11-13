import "@repo/ui/tokens.css";
import "@repo/ui/fonts.css";
import "./globals.css";
// 상태 화면(404·500) 스타일은 **여기서** 불러온다. not-found 경계가 다른 라우트에서
// 던져진 `notFound()` 로 렌더될 때 Next 는 그 라우트에 맞춰 CSS 를 링크하는데,
// not-found 청크의 CSS 는 그 목록에 들어가지 않는다 — 클래스는 붙었는데 규칙이 없는
// 맨몸 화면이 나온다(실제로 겪음). 루트 레이아웃의 CSS 는 모든 라우트에 실린다.
// 오류 화면도 같은 처지다: 렌더가 실패한 뒤에 뜨는 화면이라 더 확실히 해둬야 한다.
import "@/components/status-screen.module.css";
import type { ReactNode } from "react";
import Script from "next/script";

export const metadata = {
  title: "nginx-web-ui",
  description: "Web control for nginx",
};

// Blocking (beforeInteractive) inline script: reads the persisted theme
// (falling back to prefers-color-scheme) and sets data-theme on <html>
// before first paint, so there's no flash of the wrong theme on load.
// Kept dependency-free (no React) since it must run before hydration.
const THEME_INIT_SCRIPT = `(function () {
  try {
    document.documentElement.setAttribute("data-nav-collapsed", String(window.localStorage.getItem("nwui-nav-collapsed") === "true"));
  } catch (e) {}
  try {
    var stored = window.localStorage.getItem("theme");
    var theme =
      stored === "light" || stored === "dark"
        ? stored
        : window.matchMedia("(prefers-color-scheme: dark)").matches
          ? "dark"
          : "light";
    document.documentElement.setAttribute("data-theme", theme);
  } catch (e) {
    // localStorage/matchMedia unavailable (e.g. disabled storage) — fall
    // back to the light default already baked into :root.
  }
})();`;

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="ko">
      <head>
        {/* 이 스크립트는 **정상 응답에서만** 돈다. 404·500 응답에서는 실제 <head> 가
            거의 비어 나가고 레이아웃의 head 가 RSC 페이로드로 와서 하이드레이션 때
            DOM 에 꽂히는데, 그렇게 삽입된 인라인 스크립트는 브라우저가 실행하지
            않는다(실측: 정상 페이지 data-theme="dark", 404/500 은 null).

            그래서 그 두 화면의 다크는 스크립트가 아니라 tokens.css 의
            `prefers-color-scheme` 폴백이 책임진다. 여기를 손봐도 그 경로는 못 고친다. */}
        <Script id="theme-init" strategy="beforeInteractive">
          {THEME_INIT_SCRIPT}
        </Script>
      </head>
      <body>{children}</body>
    </html>
  );
}
