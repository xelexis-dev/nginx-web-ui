import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import type { TwoFactorState } from "../form-types";

// The card that shows fresh backup codes once, and its TXT save: the file's exact text, how the
// download is made (a Blob and a temporary link, nothing sent), what a failure says, and that the
// save never finishes the setup or changes the codes.
type FormAction = (state: TwoFactorState, form: FormData) => Promise<TwoFactorState>;
const mocks = vi.hoisted(() => ({ actions: [] as FormAction[], state: { ok: false } as TwoFactorState, submit: vi.fn() }));
vi.mock("react", async (original) => ({
  ...(await original<typeof import("react")>()),
  useActionState: (action: FormAction) => {
    mocks.actions.push(action);
    return [mocks.state, () => undefined, false];
  },
}));
vi.mock("next/navigation", () => ({ useRouter: () => ({ push: vi.fn(), refresh: vi.fn() }) }));
vi.mock("@/lib/api-transport", () => ({ submitApiForm: mocks.submit }));

import { BACKUP_CODES_FILENAME, BackupCodesCard, backupCodesText, saveBackupCodesFile, type SaveEnvironment } from "./backup-codes-card";
import { TwoFactorEnroll } from "./two-factor-forms";

const CODES = ["a1b2c-3d4e5", "00000-fffff", "12345-abcde"];
const forms = (html: string) => html.match(/<form[\s\S]*?<\/form>/g) ?? [];

beforeEach(() => {
  mocks.actions.length = 0;
  mocks.state = { ok: false };
  vi.clearAllMocks();
});

describe("the TXT file", () => {
  it("is a product heading, the single-use and safe-keeping notice, then one code per line", () => {
    expect(backupCodesText(CODES)).toBe(
      [
        "nginx-web-ui 2단계 인증 백업 코드",
        "",
        "인증 앱을 쓸 수 없을 때 로그인 화면의 「백업 코드로 인증」에서 하나씩 입력합니다.",
        "각 코드는 한 번만 쓸 수 있습니다. 이 파일은 비밀번호처럼 안전한 곳에 보관하세요.",
        "",
        "a1b2c-3d4e5",
        "00000-fffff",
        "12345-abcde",
        "",
      ].join("\n"),
    );
    expect(BACKUP_CODES_FILENAME).toBe("nginx-web-ui-backup-codes.txt");
  });
});

/** A browser stand-in that records what the save does. */
function fakeEnvironment(overrides: { createObjectURL?: () => string; click?: () => void } = {}) {
  const log: string[] = [];
  const blobs: { parts: unknown[]; type: string | undefined }[] = [];
  const anchor = {
    href: "",
    download: "",
    rel: "",
    style: { display: "" },
    click: overrides.click ?? (() => log.push("click")),
    remove: () => log.push("remove"),
  };
  const timers: (() => void)[] = [];
  const env: SaveEnvironment = {
    Blob: class {
      constructor(parts: unknown[], options?: { type?: string }) {
        blobs.push({ parts, type: options?.type });
      }
    } as unknown as typeof Blob,
    URL: {
      createObjectURL: overrides.createObjectURL ?? (() => (log.push("create"), "blob:fake/1")),
      revokeObjectURL: (url: string) => log.push(`revoke ${url}`),
    },
    document: {
      createElement: () => anchor as unknown as HTMLAnchorElement,
      body: { append: () => log.push("append") } as unknown as HTMLElement,
    },
    setTimeout: (fn: () => void) => {
      timers.push(fn);
      return 0;
    },
  };
  return { env, log, blobs, anchor, runTimers: () => timers.splice(0).forEach((fn) => fn()) };
}

describe("saving the file", () => {
  it("makes a UTF-8 text Blob, clicks a temporary download link, removes it, then revokes the URL", () => {
    const { env, log, blobs, anchor, runTimers } = fakeEnvironment();
    expect(saveBackupCodesFile(CODES, env)).toBe(true);
    expect(blobs).toEqual([{ parts: [backupCodesText(CODES)], type: "text/plain;charset=utf-8" }]);
    expect(anchor.href).toBe("blob:fake/1");
    expect(anchor.download).toBe(BACKUP_CODES_FILENAME);
    expect(log).toEqual(["create", "append", "click", "remove"]);
    // Revoked after the click, not before: the download has to start first.
    runTimers();
    expect(log).toEqual(["create", "append", "click", "remove", "revoke blob:fake/1"]);
  });

  it("reports a failure to prepare the file, leaving no link behind and nothing to revoke", () => {
    const { env, log, runTimers } = fakeEnvironment({
      createObjectURL: () => {
        throw new Error("blocked");
      },
    });
    expect(saveBackupCodesFile(CODES, env)).toBe(false);
    runTimers();
    expect(log).toEqual([]);
  });

  it("reports a failed click as a failure, still removing the link and revoking the URL", () => {
    const { env, log, runTimers } = fakeEnvironment({
      click: () => {
        throw new Error("no download");
      },
    });
    expect(saveBackupCodesFile(CODES, env)).toBe(false);
    runTimers();
    expect(log).toEqual(["create", "append", "remove", "revoke blob:fake/1"]);
  });

  it("never changes the codes it is given", () => {
    const codes = Object.freeze([...CODES]) as string[];
    const { env } = fakeEnvironment();
    expect(() => saveBackupCodesFile(codes, env)).not.toThrow();
    expect(codes).toEqual(CODES);
  });
});

describe("the card after enabling 2FA", () => {
  it.each([{}, { context: "enrollment" as const }])("shows the codes, a TXT save that is a plain button outside the finish form, and the finish form: %o", async (props) => {
    const html = renderToStaticMarkup(<BackupCodesCard codes={CODES} {...props} />);
    expect(html).toContain("2FA가 활성화되었습니다");
    for (const code of CODES) expect(html).toContain(`>${code}</code>`);
    expect(html).toMatch(/<button[^>]*type="button"[^>]*>백업 코드 TXT 저장<\/button>/);
    expect(forms(html)).toHaveLength(1);
    expect(forms(html)[0]).not.toContain("백업 코드 TXT 저장");
    expect(forms(html)[0]).toMatch(/>저장했습니다 — 계정으로</);
    const form = new FormData();
    mocks.submit.mockResolvedValue({ result: { ok: true } });
    await mocks.actions[0]!({ ok: false }, form);
    expect(mocks.submit).toHaveBeenCalledExactlyOnceWith({ id: "account.finishTwoFactorSetup", args: [] }, form);
  });

  it("has nothing to announce until a save is tried", () => {
    const html = renderToStaticMarkup(<BackupCodesCard codes={CODES} />);
    expect(html).not.toMatch(/role="alert"/);
    expect(html).toMatch(/<p[^>]*role="status"[^>]*><\/p>/);
  });

  it("is what enrolling shows once 2FA is on", () => {
    mocks.state = { ok: true, backupCodes: CODES } as TwoFactorState;
    const html = renderToStaticMarkup(<TwoFactorEnroll secret="JBSWY3DPEHPK3PXP" qrDataUrl="data:image/png;base64," />);
    expect(html).toContain("2FA가 활성화되었습니다");
    expect(html).toContain("백업 코드 TXT 저장");
    for (const code of CODES) expect(html).toContain(code);
  });
});

describe("the card after issuing new codes", () => {
  it("says the old codes stopped working, keeps the TXT save, and links back to 2FA management without finishing anything", () => {
    const html = renderToStaticMarkup(<BackupCodesCard codes={CODES} context="regenerated" />);
    expect(html).toContain("백업 코드를 새로 발급했습니다");
    expect(html).toContain("이전 백업 코드는 더 이상 쓸 수 없습니다.");
    expect(html).not.toContain("2FA가 활성화되었습니다");
    expect(html).toMatch(/<button[^>]*type="button"[^>]*>백업 코드 TXT 저장<\/button>/);
    expect(forms(html)).toHaveLength(0);
    expect(mocks.actions).toHaveLength(0);
    expect(html).toMatch(/<a[^>]*href="\/account\/2fa"[^>]*>저장했습니다 — 2FA 관리로<\/a>/);
    expect(html).not.toContain("저장했습니다 — 계정으로");
  });
});
