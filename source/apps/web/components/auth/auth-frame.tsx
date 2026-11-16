import type { ReactNode } from "react";
import { BrandLogo } from "@repo/ui";
import { APP_SEMVER } from "@/lib/version";
import styles from "./auth-screen.module.css";
import { ThemeToggle } from "@/components/theme-sync";

/** Shared product frame; the pages keep ownership of authentication state and commands. */
export function AuthFrame({
  title,
  subtitle,
  children,
  firstRun = false,
}: {
  title: ReactNode;
  subtitle: ReactNode;
  children: ReactNode;
  firstRun?: boolean;
}) {
  const note = firstRun
    ? "관리자 계정은 최초 설정에서 한 번만 만들 수 있습니다."
    : "이 설치본에 등록된 계정으로 로그인하세요.";
  return (
    <main className={styles.screen}>
      <div className={styles.frame}>
        <header className={styles.panelUtility}>
          <ThemeToggle />
        </header>
        <section className={styles.panel} aria-labelledby="auth-title">
          <div className={styles.authContent}>
            <header className={styles.heading}>
              <BrandLogo size="signIn" variant="mark" />
              <h1 id="auth-title">{title}</h1>
              <p>{subtitle}</p>
            </header>
            {children}
          </div>
        </section>
        <footer className={styles.panelFooter}>
          <span>
            nginx-web-ui <span>v{APP_SEMVER}</span>
          </span>
          <p>{note}</p>
        </footer>
      </div>
    </main>
  );
}
