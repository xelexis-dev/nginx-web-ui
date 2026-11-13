import { describe, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
const state = vi.hoisted(() => ({
  fieldErrors: { alias: "별칭 오류" },
  ok: false,
}));
vi.mock("react", async (original) => ({
  ...(await original<typeof import("react")>()),
  useActionState: () => [state, () => undefined, false],
}));
vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: vi.fn(), refresh: vi.fn() }),
}));
vi.mock("@/lib/use-api-action", () => ({ useApiAction: () => vi.fn() }));
import { CertificateForm } from "./certificate-form";
import { CertificateEditForm } from "./certificate-edit-form";
for (const node of [false, true])
  describe(node ? "node certificate forms" : "local certificate forms", () => {
    const target = node
      ? { kind: "node" as const, id: "n1", name: "운영 노드" }
      : { kind: "local" as const };
    const cancelHref = node ? "/nodes/n1/certificates" : "/certificates";
    const action = {
      id: node
        ? "node.createNodeCertificate"
        : "certificates.createCertificate",
      args: node ? ["n1"] : [],
    };
    it.each(["create", "edit"])(
      "%s preserves the labelled card, field error, native fields and submit/cancel footer",
      (kind) => {
        const html = renderToStaticMarkup(
          kind === "create" ? (
            <CertificateForm
              info="발급 설정을 바꿉니다."
              target={target}
              action={action}
              defaultEmail="owner@example.test"
              cancelHref={cancelHref}
            />
          ) : (
            <CertificateEditForm
              info="발급 설정을 바꿉니다."
              target={target}
              action={{
                ...action,
                id: node
                  ? "node.updateNodeCertificate"
                  : "certificates.updateCertificate",
              }}
              id="c1"
              cancelHref={cancelHref}
              isIssued
              hasStoredToken
              defaults={{
                alias: "운영 인증서",
                domains: ["api.example.test"],
                email: "owner@example.test",
                staging: false,
                challengeType: "dns-01",
                dnsProvider: "cloudflare",
              }}
            />
          ),
        );
        expect(html.match(/<footer\b/g)).toHaveLength(1);
        expect(html).toContain(
          'aria-labelledby="certificate-verification-heading"',
        );
        expect(html).toContain('aria-describedby="cert-challenge-hint"');
        if (kind === "edit")
          expect(html).toContain(
            'aria-labelledby="certificate-target-heading"',
          );
        expect(html).toContain('data-tone="info"');
        expect(html.indexOf("발급 설정을 바꿉니다.")).toBeLessThan(
          html.indexOf('<section id="certificate-settings"'),
        );
        expect(html).toContain(
          'aria-labelledby="certificate-settings-heading"',
        );
        expect(html).toContain('data-error=""');
        expect(html).toContain("별칭 오류");
        expect(html).toContain('role="alert"');
        expect(html).toContain('href="#cert-alias"');
        expect(html.match(/<button[^>]*type="submit"/g)).toHaveLength(1);
        expect(html).toContain('href="' + cancelHref + '"');
        expect(html).toContain('name="email"');
        expect(html).toContain('name="staging" value="false"');
        expect(html).toContain(
          'name="challengeType" value="' +
            (kind === "create" ? "http-01" : "dns-01") +
            '"',
        );
        if (kind === "edit") {
          expect(html).toContain('name="id" value="c1"');
          expect(html).toContain('name="dnsToken" value="__keep__"');
        }
        if (node) expect(html).toContain("저장 대상: 운영 노드");
      },
    );
  });
