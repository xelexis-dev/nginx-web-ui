"use client";

import { useEffect, useState } from "react";
import styles from "./theme-sync.module.css";
import { Icon, IconButton, applyTheme, type Theme } from "@repo/ui";

/**
 * 상태 화면(404·500)에서만 쓰는 테마 보정.
 *
 * 이 화면들은 루트 레이아웃의 테마 스크립트가 실행되지 않는 응답으로 나간다
 * (레이아웃의 head 가 RSC 페이로드로 와서 하이드레이션 때 DOM 에 꽂히는데, 그렇게
 * 삽입된 인라인 스크립트는 브라우저가 실행하지 않는다 — 실측). 그래서 첫 페인트는
 * tokens.css 의 `prefers-color-scheme` 폴백, 즉 **OS 선호**로 그려진다.
 *
 * 문제는 사용자가 앱에서 OS 와 다른 테마를 골라둔 경우다. 다크를 골라둔 사람이
 * 라이트 OS 를 쓰면 404 가 흰 화면으로 뜬다 — 토글을 한 번이라도 쓴 사용자는 전부
 * 해당된다. 하이드레이션 직후 저장된 선택을 반영해 그 간극만 닫는다.
 *
 * 정상 페이지에는 넣지 않는다. 거기서는 스크립트가 첫 페인트 전에 이미 돈다.
 */
export function ThemeSync() {
  useEffect(() => {
    try {
      const stored = window.localStorage.getItem("theme");
      if (stored !== "light" && stored !== "dark") return;
      if (document.documentElement.getAttribute("data-theme") === stored)
        return;
      document.documentElement.setAttribute("data-theme", stored);
    } catch {
      // localStorage 를 막아둔 환경 — OS 선호로 그려진 그대로 둔다.
    }
  }, []);

  return null;
}

/** Utility-bar and auth-frame toggle share the existing theme storage contract. */
export function ThemeToggle() {
  const [theme, setTheme] = useState<Theme>("light");
  useEffect(() => {
    const read = () =>
      setTheme(
        document.documentElement.dataset.theme === "dark" ||
          (!document.documentElement.dataset.theme &&
            window.matchMedia("(prefers-color-scheme: dark)").matches)
          ? "dark"
          : "light",
      );
    read();
    const observer = new MutationObserver(read);
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-theme"],
    });
    return () => observer.disconnect();
  }, []);
  const next = theme === "light" ? "dark" : "light";
  return (
    <IconButton
      label={`${next === "dark" ? "다크" : "라이트"} 테마로 전환`}
      variant="ghost"
      size="sm"
      className={styles.toggle}
      onClick={() => {
        applyTheme(next);
        setTheme(next);
        try {
          window.localStorage.setItem("theme", next);
        } catch {
          /* The applied theme still works when storage is unavailable. */
        }
      }}
    >
      <Icon name={next === "dark" ? "moon" : "sun"} size={16} />
    </IconButton>
  );
}
