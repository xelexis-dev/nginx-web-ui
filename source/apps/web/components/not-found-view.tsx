import { buttonClassName } from "@repo/ui";
import Link from "next/link";
import styles from "./status-screen.module.css";
import { ThemeSync } from "./theme-sync";

export interface NotFoundAction {
  href: string;
  label: string;
  primary?: boolean;
}

export interface NotFoundViewProps {
  title: string;
  description: string;
  actions: NotFoundAction[];
  /**
   * 앱 셸(사이드바) 밖에서 단독으로 뜨는가. 알 수 없는 주소로 들어온 경우가 그렇다 —
   * 셸이 없으니 화면 가운데에 세운다. 셸 안(항목을 못 찾은 경우)에서는 이미 여백과
   * 폭이 잡혀 있어 그대로 흐른다.
   */
  standalone?: boolean;
}

/**
 * 404 화면. 세 진입점(알 수 없는 주소 / 앱 안의 없는 항목 / 관리자 패널의 없는 항목)이
 * 같은 화면을 쓰고 문구와 이동 경로만 다르다.
 */
export function NotFoundView({
  title,
  description,
  actions,
  standalone,
}: NotFoundViewProps) {
  const card = (
    <div className={styles.card}>
      <ThemeSync />
      <div className={styles.protocol}>HTTP/1.1</div>
      <p className={styles.status}>404 Not Found</p>
      <div className={styles.rule} />
      <h1 className={styles.title}>{title}</h1>
      <p className={styles.description}>{description}</p>
      <div className={styles.actions}>
        {actions.map((action) => (
          <Link
            key={action.href}
            href={action.href}
            className={buttonClassName({
              variant: action.primary ? "primary" : "secondary",
            })}
          >
            {action.label}
          </Link>
        ))}
      </div>
    </div>
  );

  return standalone ? <div className={styles.standalone}>{card}</div> : card;
}
