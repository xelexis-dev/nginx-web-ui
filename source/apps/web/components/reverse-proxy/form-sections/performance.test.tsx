import { expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { CommonView } from "../form-views/common-view";
import { emptyDirectiveState } from "@repo/presentation/nginx/directive-state";

it("identifies legacy server gzip instead of showing it as off", () => {
  const html = renderToStaticMarkup(<CommonView legacyGzip onCacheChange={() => {}} onGzipChange={() => {}} serverDirectives={emptyDirectiveState()} onServerDirectivesChange={()=>{}} locations={[]} onRoute={()=>{}} />);
  const control = html;
  expect(control).toContain("고급 설정 사용");
});
