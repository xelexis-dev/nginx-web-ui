import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";
import { describe, expect, it } from "vitest";

const packageRoot = fileURLToPath(new URL("..", import.meta.url));
const workspaceRoot = path.resolve(packageRoot, "../..");
const readJson = (file: string) => JSON.parse(readFileSync(file, "utf8"));

describe("frontend presentation boundary", () => {
  it("has no production dependencies or external implementation imports", () => {
    const manifest = readJson(path.join(packageRoot, "package.json"));
    expect(Object.keys(manifest.dependencies ?? {})).toEqual([]);
    const files = readdirSync(path.join(packageRoot, "src"), { recursive: true })
      .map(String).filter((file) => file.endsWith(".ts") && !file.endsWith(".test.ts"));
    expect(files.length).toBeGreaterThan(10);
    for (const file of files) {
      const ast = ts.createSourceFile(file, readFileSync(path.join(packageRoot, "src", file), "utf8"), ts.ScriptTarget.Latest, true);
      const visit = (node: ts.Node) => {
        if ((ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) && node.moduleSpecifier && ts.isStringLiteral(node.moduleSpecifier)) {
          expect(node.moduleSpecifier.text.startsWith("."), `${file}: ${node.moduleSpecifier.text}`).toBe(true);
        }
        if (ts.isCallExpression(node) && (node.expression.kind === ts.SyntaxKind.ImportKeyword || (ts.isIdentifier(node.expression) && ["fetch", "require"].includes(node.expression.text)))) {
          throw new Error(`${file} must not load runtime modules or perform transport`);
        }
        ts.forEachChild(node, visit);
      };
      visit(ast);
    }
  });

  it("keeps backend workspace packages outside web's production dependency closure", () => {
    const manifests = new Map<string, { dependencies?: Record<string, string> }>();
    for (const group of ["apps", "packages"]) {
      for (const name of readdirSync(path.join(workspaceRoot, group))) {
        try {
          const manifest = readJson(path.join(workspaceRoot, group, name, "package.json"));
          manifests.set(manifest.name, manifest);
        } catch { /* Rust-only applications have no package.json. */ }
      }
    }
    const found = new Set<string>();
    const visit = (name: string) => {
      if (found.has(name)) return;
      found.add(name);
      for (const dependency of Object.keys(manifests.get(name)?.dependencies ?? {})) visit(dependency);
    };
    visit("web");
    for (const name of ["@repo/db", "@repo/auth", "@repo/crypto", "@repo/acme", "@repo/nginx", "@repo/node-api", "@repo/webhook", "nodemailer"]) {
      expect(found.has(name), name).toBe(false);
    }
    expect(found.has("@repo/presentation")).toBe(true);
  });
});
