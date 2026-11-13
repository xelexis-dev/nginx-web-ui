import path from "node:path";
import { defineConfig } from "vitest/config";

// apps/web 의 첫 테스트 파일(node-client.test.ts)이 이 설정을 필요로 한다.
// tsconfig.json 의 `"@/*": ["./*"]` 와 같은 별칭을 vitest 에도 그대로 준다 —
// vitest 는 tsconfig paths 를 자동으로 읽지 않는다.
export default defineConfig({
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "."),
    },
  },
  // `lib/node-page.test.tsx`(Task 2)가 이 저장소의 첫 `.tsx` 테스트 파일이다.
  // `tsconfig.json` 의 `"jsx": "preserve"` 는 Next 자체 SWC 변환을 위한 값이고,
  // Vite 8 은 기본 변환기가 esbuild 가 아니라 oxc 다(vitest 가 이미 내부적으로
  // `oxc` 를 채워 두므로 `esbuild.*` 를 여기 적어도 조용히 무시된다 — Vite 가
  // "Both esbuild and oxc options were set" 로 경고하고 esbuild 쪽을 버린다).
  // 그래서 `oxc.jsx` 를 직접 지정해야 한다: 지정하지 않으면 oxc 가 파일 근처
  // tsconfig 를 읽어 `"preserve"` 를 그대로 따라 JSX 가 JS 로 바뀌지 않고,
  // 그 뒤 import-analysis 단계가 "invalid JS syntax" 로 파싱에 실패한다.
  oxc: {
    jsx: { runtime: "automatic" },
  },
  // `next@16.3.0` 은 package.json 에 `exports` 필드가 없다. next-auth 의
  // lib/env.js 는 확장자 없이 `import ... from "next/server"` 를 쓰는데,
  // `exports` 가 없으면 엄격 ESM 로더(Vite 의 SSR 외부화 경로가 그대로
  // 넘기는 Node 네이티브 로더 포함)가 확장자를 자동으로 붙여주지 않아
  // 그 임포트가 풀리지 않는다(`node -e "import('next-auth')"` 로 vitest 와
  // 무관하게 재현됨). `noExternal` 로 next-auth 를 Vite 번들링 경로에 넣으면
  // Vite 자체 리졸버가 확장자를 찾아 붙여 해결된다. next 가 `exports` 필드를
  // 갖는 버전으로 오르면 이 줄은 지워도 된다 — 그때 다시 깨지는지 확인할 것.
  ssr: {
    noExternal: ["next-auth"],
  },
  test: {
    /**
     * **검사의 타임존을 고정한다. 그리고 UTC 로 고정하지 않는다.**
     *
     * 고정하지 않으면 이 스위트의 TZ 는 「그 기계의 TZ」다 — 결과가 사람마다 다르고
     * 아무도 그걸 모른다. 그런데 **UTC 로 고정하면 더 나쁘다**: TZ 의존 결함이
     * 영원히 안 보이게 된다.
     *
     * 실측(2026-08-30, `app/(app)/account/tokens`): 만료 날짜를
     * `` `${raw}T23:59:59.999Z` `` 로 파싱하도록 고친 뒤 **`Z` 를 뺀 변이**를 넣었더니
     * 이 기계(Asia/Seoul)에서는 3검사가 죽었는데 **`TZ=UTC` 에서는 74/74 로
     * 살아남았다.** 기댓값이 절대 시각이라 **UTC 아닌 러너만이 둘을 구별한다** —
     * 그리고 컨테이너도 CI 러너도 UTC 다. 즉 그 한 글자를 지키던 것은 검사가 아니라
     * 개발자 기계의 타임존이었다.
     *
     * `Asia/Seoul` 인 이유는 둘이다: 이 제품의 로케일이고, **DST 가 없어** 오프셋이
     * 계절에 따라 흔들리지 않는다(그랬다면 검사가 날짜에 따라 색이 변한다).
     *
     * ⚠ **이 값을 UTC 로 바꾸지 마라.** 초록은 그대로지만 TZ 축이 통째로 눈을 감는다.
     */
    env: { TZ: "Asia/Seoul" },
    /**
     * **`.next` 를 반드시 뺀다.** `next build` 의 standalone 출력이
     * `.next/standalone/apps/web/**` 에 소스 트리를 통째로 복사하고, 거기에는
     * `*.test.ts(x)` 도 함께 들어간다. 빼지 않으면 vitest 가 원본과 사본을 둘 다
     * 수집해 **통과 수가 정확히 두 배로 부풀고**(55 → 110), 그 숫자가 「검사가
     * 늘었다」로 읽힌다. 실제로 이 저장소에서 두 번 그랬다.
     *
     * 빌드 산출물을 그때그때 지우는 것으로는 못 막는다 — `yarn build` 를 돌릴
     * 때마다 다시 생기고, 그걸 기억하는 사람에게 달린 문제가 된다. 여기서
     * 구조적으로 끊는다.
     *
     * vitest 기본 exclude(`node_modules`/`dist` 등)를 덮어쓰는 것이므로 기본값도
     * 함께 적는다 — 빠뜨리면 `node_modules` 안의 테스트까지 수집된다.
     */
    exclude: ["**/node_modules/**", "**/dist/**", "**/.next/**"],
  },
});
