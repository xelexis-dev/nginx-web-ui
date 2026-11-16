"use client";

import { useState } from "react";
import Link from "next/link";
import { Surface, CodePane, Button, buttonClassName, Banner } from "@repo/ui";
import { ApiForm } from "@/components/api-form";
import styles from "./two-factor.module.css";

/**
 * Freshly issued backup codes, shown once: after enabling 2FA ("enrollment": the finish form ends
 * the setup) or after issuing new codes from 2FA management ("regenerated": a link back to it, no
 * action). Either way the codes can be saved as a .txt file. The file is made here in the browser from
 * the codes on screen (a Blob behind a temporary download link): nothing is sent, stored or logged,
 * and the codes are neither changed nor hidden by saving.
 */

export const BACKUP_CODES_FILENAME = "nginx-web-ui-backup-codes.txt";
/** The object URL is revoked this long after the click, once the browser has taken the download. */
const REVOKE_AFTER_MS = 1_000;

/** The file's text: a heading, what the codes are for and how to keep them, then one code per line. */
export function backupCodesText(codes: readonly string[]): string {
  return [
    "nginx-web-ui 2단계 인증 백업 코드",
    "",
    "인증 앱을 쓸 수 없을 때 로그인 화면의 「백업 코드로 인증」에서 하나씩 입력합니다.",
    "각 코드는 한 번만 쓸 수 있습니다. 이 파일은 비밀번호처럼 안전한 곳에 보관하세요.",
    "",
    ...codes,
    "",
  ].join("\n");
}

/** What saving needs from the browser; tests pass a stand-in. */
export interface SaveEnvironment {
  Blob: typeof Blob;
  URL: Pick<typeof URL, "createObjectURL" | "revokeObjectURL">;
  document: {
    createElement(tag: "a"): HTMLAnchorElement;
    body: Pick<HTMLElement, "append">;
  };
  setTimeout: (handler: () => void, ms: number) => unknown;
}

const browser = (): SaveEnvironment => ({
  Blob,
  URL,
  document,
  setTimeout: (handler, ms) => window.setTimeout(handler, ms),
});

/**
 * Starts a download of the codes as a UTF-8 text file. `true` once the browser was handed the
 * download; `false` if the file could not be prepared (the codes are still on screen to copy). It
 * cannot know whether the person kept the file, so it never says so.
 */
export function saveBackupCodesFile(
  codes: readonly string[],
  env: SaveEnvironment = browser(),
): boolean {
  let url: string | null = null;
  let link: HTMLAnchorElement | null = null;
  try {
    url = env.URL.createObjectURL(
      new env.Blob([backupCodesText(codes)], {
        type: "text/plain;charset=utf-8",
      }),
    );
    link = env.document.createElement("a");
    link.href = url;
    link.download = BACKUP_CODES_FILENAME;
    link.rel = "noopener";
    link.style.display = "none";
    env.document.body.append(link);
    link.click();
    return true;
  } catch {
    return false;
  } finally {
    link?.remove();
    if (url) {
      const made = url;
      env.setTimeout(() => env.URL.revokeObjectURL(made), REVOKE_AFTER_MS);
    }
  }
}

export interface BackupCodesCardProps {
  codes: string[];
  /** Why the codes are shown. Default "enrollment". */
  context?: "enrollment" | "regenerated";
}

const COPY = {
  enrollment: {
    title: "2FA가 활성화되었습니다",
    warning: (
      <>
        아래 <strong>백업 코드</strong>를 안전한 곳에 보관하세요. 인증 앱을
        사용할 수 없을 때 로그인에 사용하며, 각 코드는 <strong>한 번만</strong>{" "}
        쓸 수 있습니다. 이 화면을 벗어나면 다시 볼 수 없습니다.
      </>
    ),
  },
  regenerated: {
    title: "백업 코드를 새로 발급했습니다",
    warning: (
      <>
        이전 백업 코드는 더 이상 쓸 수 없습니다. 아래 <strong>새 코드</strong>를
        안전한 곳에 보관하세요. 각 코드는 <strong>한 번만</strong> 쓸 수 있고,
        이 화면을 벗어나면 다시 볼 수 없습니다.
      </>
    ),
  },
} as const;

export function BackupCodesCard({
  codes,
  context = "enrollment",
}: BackupCodesCardProps) {
  const [saved, setSaved] = useState<"idle" | "started" | "failed">("idle");
  const copy = COPY[context];
  return (
    <Surface id="two-factor-backup-codes" title={copy.title}>
      <div className={styles.backupCard}>
        <p className={styles.warn}>{copy.warning}</p>
        <CodePane title="백업 코드" variant="secret" code={codes.join("\n")} />

        {/* Outside every form and type=button: saving never finishes the setup or sends anything. */}
        <div className={styles.saveRow}>
          <Button
            type="button"
            variant="secondary"
            onClick={() =>
              setSaved(saveBackupCodesFile(codes) ? "started" : "failed")
            }
          >
            백업 코드 TXT 저장
          </Button>
          {/* Always present so the change is announced; it says the download started, not that a file was kept. */}
          <p className={styles.saveNote} role="status">
            {saved === "started"
              ? `${BACKUP_CODES_FILENAME} 내려받기를 시작했습니다. 브라우저의 다운로드에서 확인하세요.`
              : ""}
          </p>
        </div>
        {saved === "failed" ? (
          <Banner tone="error" role="alert" title="파일을 만들지 못했습니다.">
            위 코드를 직접 옮겨 적어 보관하세요.
          </Banner>
        ) : null}

        {context === "enrollment" ? (
          <ApiForm action={{ id: "account.finishTwoFactorSetup", args: [] }}>
            <Button type="submit" className={styles.doneAction}>
              저장했습니다 — 계정으로
            </Button>
          </ApiForm>
        ) : (
          <Link
            href="/account/2fa"
            className={buttonClassName({ className: styles.doneAction })}
          >
            저장했습니다 — 2FA 관리로
          </Link>
        )}
      </div>
    </Surface>
  );
}
