import { readdirSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { expect, it } from "vitest";

// Unknown CSS variables invalidate the whole declaration in the browser (not just its value).
// Catch the previous --surface-padding class of defect across the web and shared UI.
it("every required CSS variable has a definition", () => {
  const roots = [resolve(import.meta.dirname, ".."), resolve(import.meta.dirname, "../../../packages/ui/src"), resolve(import.meta.dirname, "../../storybook/src"), resolve(import.meta.dirname, "../../../docker/upgrade-status")];
  const sources: { path: string; text: string }[] = [];
  const visit = (dir: string) => {
    for (const item of readdirSync(dir, { withFileTypes: true })) {
      if (item.name.startsWith(".") || item.name === "node_modules") continue;
      const path = join(dir, item.name);
      if (item.isDirectory()) visit(path);
      else if (/\.(css|tsx|mdx)$/.test(item.name) && !item.name.includes(".test.")) {
        sources.push({ path, text: readFileSync(path, "utf8").replace(/\/\*[\s\S]*?\*\//g, "").replace(/^\s*\/\/.*$/gm, "") });
      }
    }
  };
  roots.forEach(visit);
  const definitions = new Set(sources.flatMap(({ text }) => [...text.matchAll(/(--[\w-]+)["']?\s*:/g)].map((m) => m[1])));
  const missing = sources.flatMap(({ path, text }) => [...text.matchAll(/var\(\s*(--[\w-]+)\s*\)/g)]
    .filter((m) => !definitions.has(m[1])).map((m) => `${path}: ${m[1]}`));
  expect(missing).toEqual([]);
});
