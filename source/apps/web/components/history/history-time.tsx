"use client";

import { useEffect, useState } from "react";

// 브라우저 시간대를 못 구하면 KST(한국 표준시)로 표시한다.
const FALLBACK_TZ = "Asia/Seoul";

function format(iso: string, timeZone: string): string {
  return new Intl.DateTimeFormat("ko-KR", { dateStyle: "medium", timeStyle: "medium", timeZone }).format(new Date(iso));
}

/**
 * History 시각을 "현재 브라우저의 시간대"로 표시한다(형식은 ko-KR 고정, 시간대만
 * 로컬라이즈). 서버 컴포넌트는 브라우저 시간대를 알 수 없으므로 UTC ISO 문자열만
 * 내려주고, 시간대 변환은 여기(클라이언트)에서 한다.
 *
 * 시간대는 `Intl.DateTimeFormat().resolvedOptions().timeZone`(브라우저 OS의 로컬
 * IANA 존 = 사용자 국가 반영). 못 구하면 KST로 폴백한다.
 *
 * 하이드레이션 mismatch를 피하려고 초기 렌더는 KST로(서버 SSR과 첫 클라이언트
 * 렌더가 동일한 문자열) 그린 뒤, mount 후 실제 브라우저 시간대로 교체한다.
 *
 * `components/history/`(라우트 폴더가 아니라)에 두는 이유는 `history-list.tsx`
 * 와 같다 — `(app)/history` 와 `(node)/nodes/[id]/history` 양쪽이 이 컴포넌트를
 * 쓴다.
 */
export function HistoryTime({ iso }: { iso: string }) {
  const [text, setText] = useState(() => format(iso, FALLBACK_TZ));

  useEffect(() => {
    const tz = Intl.DateTimeFormat().resolvedOptions().timeZone || FALLBACK_TZ;
    setText(format(iso, tz));
  }, [iso]);

  return (
    <time dateTime={iso} suppressHydrationWarning>
      {text}
    </time>
  );
}
