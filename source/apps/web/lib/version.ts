/**
 * 앱 버전 표시용 상수. 두 값 모두 **빌드 타임**에 next.config.ts의 `env`로 주입되므로
 * (런타임 파일/환경 읽기 없음) client 컴포넌트에서도 그대로 쓸 수 있다.
 *
 * - `APP_SEMVER` — `apps/web/package.json`의 version. 사람이 릴리스 때 bump.
 * - `FULL_SHA`   — 이미지 빌드 시 `--build-arg GIT_SHA=<sha>`로 들어온 커밋 sha(CI가 전달).
 *                  로컬 dev나 build-arg 없는 빌드에선 빈 문자열.
 *
 * 표시 규칙: `v<semver> (<sha 7자>)`, sha가 없으면 `(dev)`. 짧은 sha는 레지스트리 태그
 * `sha-<short>`와 같은 자리수라 UI에서 본 값으로 배포본을 바로 특정할 수 있다.
 */

const SHORT_SHA_LENGTH = 7;

/** 커밋 sha를 레지스트리 태그와 같은 7자로 줄인다. 빈 값/공백은 빈 문자열. */
export function shortSha(sha: string): string {
  return sha.trim().slice(0, SHORT_SHA_LENGTH);
}

/** UI에 넣을 sha 라벨. sha가 주입되지 않은 빌드는 "dev". */
export function shaLabel(sha: string): string {
  return shortSha(sha) || "dev";
}

export const APP_SEMVER = process.env.APP_SEMVER || "0.0.0";
export const FULL_SHA = process.env.APP_GIT_SHA || "";
export const SHA_LABEL = shaLabel(FULL_SHA);
