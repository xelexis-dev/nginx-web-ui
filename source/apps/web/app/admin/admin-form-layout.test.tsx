import { describe, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { FormErrors } from "@/components/form-errors";
import { EditUserForms } from "./users/[id]/edit/edit-user-forms";
import { BaseUrlForm } from "./settings/general/base-url-form";
import { MetricsForm } from "./settings/general/metrics-form";

vi.mock("@/lib/use-api-action", () => ({
  useApiAction: () => async () => ({ ok: false }),
}));

describe("admin independent forms and error summary", () => {
  it("keeps user edit submissions independent and omits the OAuth-only password form", () => {
    for (const hasPassword of [true, false]) {
      const html = renderToStaticMarkup(
        <EditUserForms
          id="user-id"
          email="operator@example.test"
          currentRole="editor"
          hasPassword={hasPassword}
        />,
      );
      expect(html.match(/<form\b/g)).toHaveLength(hasPassword ? 2 : 1);
      expect(html.match(/type="submit"/g)).toHaveLength(hasPassword ? 2 : 1);
      expect(html).not.toContain("nwui-btn-primary");
      expect(html).not.toContain("formBar");
      expect(html).toContain('name="id" value="user-id"');
      expect(html.includes('name="password"')).toBe(hasPassword);
      expect(html.includes("이 계정은 OAuth 로그인만 사용합니다")).toBe(
        !hasPassword,
      );
    }
  });
  it("keeps general settings as two secondary forms with the original defaults", () => {
    const html = renderToStaticMarkup(
      <>
        <BaseUrlForm defaultValue="https://example.test" />
        <MetricsForm
          defaultRetentionDays="30"
          defaultExcludedHosts="admin.example.test"
        />
      </>,
    );
    expect(html.match(/<form\b/g)).toHaveLength(2);
    expect(html.match(/type="submit"/g)).toHaveLength(2);
    expect(html).not.toContain("nwui-btn-primary");
    expect(html).not.toContain("formBar");
    expect(html).toContain('name="publicBaseUrl"');
    expect(html).toContain('value="https://example.test"');
    expect(html).toContain('name="metricsRetentionDays"');
    expect(html).toContain('value="30"');
    expect(html).toContain("admin.example.test");
  });
  it("links all nonempty validation messages without rendering an empty alert", () => {
    expect(renderToStaticMarkup(<FormErrors />)).toBe("");
    const html = renderToStaticMarkup(
      <FormErrors
        errors={{ email: "이메일 오류", role: "역할 오류", empty: "" }}
      />,
    );
    expect(html).toContain('role="alert"');
    expect(html.match(/type="button"/g)).toHaveLength(2);
    expect(html).toContain("이메일 오류");
    expect(html).toContain("역할 오류");
    expect(html).not.toContain('type="submit"');
  });
});
