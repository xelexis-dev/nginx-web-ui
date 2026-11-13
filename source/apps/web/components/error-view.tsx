"use client";
import { Button, buttonClassName } from "@repo/ui";

import Link from "next/link";
import styles from "./status-screen.module.css";
import { ThemeSync } from "./theme-sync";

export interface ErrorViewProps {
  /** 다시 그려보기. `catchError` 가 넘겨주는 retry — 서버 컴포넌트까지 다시 요청한다. */
  onRetry?: () => void;
  /**
   * 서버가 남긴 오류 ID. 운영자는 이 값으로 컨테이너 로그에서 실제 스택을 찾는다.
   * 클라이언트에서 난 오류에는 없다.
   */
  digest?: string;
  /** 돌아갈 곳. 앱 셸 안이면 대시보드, 관리자 패널이면 패널 첫 화면. */
  homeHref?: string;
  homeLabel?: string;
  /** 앱 셸 밖에서 단독으로 뜨는가(루트 레이아웃까지 실패한 경우). */
  standalone?: boolean;
  /**
   * 배포가 교체돼 이 화면이 오래된 빌드에서 열린 경우.
   *
   * 컨테이너를 바꾸면 이전 빌드의 Server Action ID 가 새 빌드에 없다. 그러면
   * 교체 전에 열어둔 탭에서 「저장」을 눌러도 서버가 그 액션을 찾지 못하고,
   * 화면에는 **아무 일도 일어나지 않은 것처럼** 보인다(운영 서버에서 실제로
   * 발생 — 사용자가 저장에 실패한 줄 몰랐다).
   *
   * 그때는 오류가 아니라 **원인과 해결책**을 말해야 한다. 「다시 시도」는 소용이
   * 없다 — 같은 오래된 액션을 다시 부를 뿐이라, 새로고침만이 답이다.
   */
  staleDeployment?: boolean;
}

/**
 * 500 화면. 404(`NotFoundView`)와 같은 카드·상태줄을 쓰는 형제 화면이고,
 * 다른 점은 셋뿐이다 — 상태줄 색, 오류 ID, 그리고 "다시 시도" 버튼.
 *
 * 원문 오류 메시지는 **일부러 보여주지 않는다.** 서버 컴포넌트 오류는 Next 가
 * 어차피 가려서 내려보내고(digest 만 남는다), 그 자리에 개발자용 영문 문장을
 * 띄우면 사용자는 할 수 있는 게 없다. 대신 로그로 이어지는 끈(오류 ID)을 준다.
 */
export function ErrorView({
  onRetry,
  digest,
  homeHref,
  homeLabel,
  standalone,
  staleDeployment,
}: ErrorViewProps) {
  const card = (
    <div className={styles.card}>
      <ThemeSync />
      <div className={styles.protocol}>HTTP/1.1</div>
      <p className={`${styles.status} ${styles.statusError}`}>
        {/* 실제 응답 코드를 적는다. 오래된 액션에 Next 가 내는 것은 404 +
            `x-nextjs-action-not-found: 1` 이다. 처음엔 뜻이 통한다고 409 로
            적었는데, 그것 때문에 운영 쪽에서 access.log 의 409 를 찾다가
            검증에 실패했다 — 화면의 상태줄은 장식이 아니라 로그와 맞춰
            보는 값이다. */}
        {staleDeployment ? "404 Not Found" : "500 Internal Server Error"}
      </p>
      <div className={styles.rule} />
      <h1 className={styles.title}>
        {staleDeployment
          ? "새 버전이 배포되었습니다"
          : "화면을 불러오지 못했습니다"}
      </h1>
      <p className={styles.description}>
        {staleDeployment
          ? "이 화면은 이전 버전에서 열린 것이라 방금 누른 작업은 처리되지 않았습니다. 새로고침한 뒤 다시 시도하세요. 입력하던 내용은 사라지니 필요하면 먼저 복사해 두세요."
          : "다시 시도하면 대부분 해결됩니다. 계속 같은 화면이 뜨면 아래 오류 ID로 서버 로그를 확인하세요."}
      </p>
      {digest ? (
        <p className={styles.digest}>
          <span className={styles.digestLabel}>오류 ID</span>
          <span className={styles.digestValue}>{digest}</span>
        </p>
      ) : null}
      <div className={styles.actions}>
        {/* 오래된 배포에서는 「다시 시도」가 같은 오래된 액션을 다시 부를 뿐이라
            소용이 없다. 새로고침만 낸다. */}
        {staleDeployment ? (
          <button
            type="button"
            onClick={() => window.location.reload()}
            className={buttonClassName()}
          >
            새로고침
          </button>
        ) : null}
        {!staleDeployment && onRetry ? (
          <Button type="button" onClick={() => onRetry()}>
            다시 시도
          </Button>
        ) : null}
        {homeHref ? (
          <Link
            href={homeHref}
            className={buttonClassName({ variant: "secondary" })}
          >
            {homeLabel ?? "처음으로"}
          </Link>
        ) : null}
      </div>
    </div>
  );

  return standalone ? <div className={styles.standalone}>{card}</div> : card;
}
