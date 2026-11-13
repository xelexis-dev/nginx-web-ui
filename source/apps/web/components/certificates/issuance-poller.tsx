"use client";

import { useEffect } from "react";
import { useRouter } from "next/navigation";
import { startVisibleInterval } from "@/lib/visible-interval";

/**
 * 발급/검증이 도는 동안 탭이 보이면 2초마다 서버 데이터를 재조회한다(끝나면 렌더가 이
 * 컴포넌트를 제거해 폴링이 멎는다).
 *
 * **노드 화면에도 그대로 쓰인다** — `router.refresh()` 는 하드코딩된 fetch 가
 * 아니라 **서버 컴포넌트를 다시 렌더하라**는 요청이라, 그 화면이 로컬 DB 를
 * 읽든 노드를 부르든 같은 것이 다시 돈다. 중계 라우트가 필요 없는 이유다.
 *
 * **폴링해도 되는 종점인지 확인하고 쓸 것.** 노드 인증서 화면이 도는 것은
 * `/api/v1/certificates`(평범한 DB 읽기)라 괜찮다. `/api/v1/status` 는 `nginx -t`
 * 로 자식 프로세스를 띄우고 그 라우트가 「게이트웨이가 타이머로 폴링하지 않는
 * 것」을 명시적 전제로 달아 뒀다 — **거기에 이 컴포넌트를 붙이지 마라.**
 */
export function IssuancePoller() {
  const router = useRouter();
  useEffect(() => startVisibleInterval(() => router.refresh(), 2000), [router]);
  return null;
}
