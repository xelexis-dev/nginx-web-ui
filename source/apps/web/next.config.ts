import { readFileSync } from "node:fs";
import path from "node:path";
import type { NextConfig } from "next";

const monorepoRoot = path.resolve(__dirname, "..", "..");

// 제품 버전은 이 워크스페이스의 package.json이 단일 진실(릴리스 때 수동 bump). 여기서 읽어
// 주입하므로 버전 문자열이 코드에 중복되지 않는다. 빌드 sha는 이미지 빌드 때
// `--build-arg GIT_SHA=<sha>`로 들어오고(CI가 github.sha 전달), 없으면 빈 문자열 →
// UI가 "dev"로 표시한다.
const { version: appSemver } = JSON.parse(
  readFileSync(path.join(__dirname, "package.json"), "utf8"),
) as { version: string };

const nextConfig: NextConfig = {
  output: "standalone",
  poweredByHeader: false,
  // next/image is unused; omit its optimizer and sharp/libvips from the image.
  images: { unoptimized: true },
  async headers() {
    return [{
      source: "/:path*",
      headers: [
        { key: "X-Content-Type-Options", value: "nosniff" },
        { key: "Referrer-Policy", value: "same-origin" },
      ],
    }];
  },
  outputFileTracingRoot: monorepoRoot,
  // Turbopack (default builder in Next 16) infers the monorepo root separately
  // from outputFileTracingRoot; without this it mis-resolves the app dir. See
  // https://nextjs.org/docs/app/api-reference/config/next-config-js/turbopack#root-directory
  turbopack: {
    root: monorepoRoot,
  },
  // Workspace packages ship raw TS/TSX from src/, so Next must transpile them.
  transpilePackages: ["@repo/ui", "@repo/presentation"],
  // `env` 키는 빌드 시 값을 번들에 리터럴로 치환한다 — NEXT_PUBLIC_ 접두어 없이도
  // 클라이언트 번들까지 들어가므로, client 컴포넌트인 Sidebar가 lib/version.ts를
  // 그대로 import해 쓸 수 있다(런타임에 파일/env를 읽지 않는다).
  env: {
    APP_SEMVER: appSemver,
    APP_GIT_SHA: process.env.GIT_SHA ?? "",
  },
};

export default nextConfig;
