export type { ConfigPreview } from "@repo/contracts";
import type { ConfigPreview } from "@repo/contracts";
/** Rendered by the resource owner; older nodes explicitly report unavailable. */
export function configPreview(value: unknown): ConfigPreview {
  if (value == null) return { config: null, error: "이 노드는 설정 미리보기를 제공하지 않습니다." };
  if (typeof value !== "object") throw new Error("Invalid configuration preview");
  const preview = value as ConfigPreview;
  if ((typeof preview.config === "string" && preview.error === null) ||
      (preview.config === null && typeof preview.error === "string" && preview.error.length > 0)) {
    const { unresolvedPaths, ...rest } = preview;
    return Array.isArray(unresolvedPaths) && unresolvedPaths.every(path => typeof path === "string" && path.length > 0)
      ? { ...rest, unresolvedPaths }
      : rest;
  }
  throw new Error("Invalid configuration preview");
}
