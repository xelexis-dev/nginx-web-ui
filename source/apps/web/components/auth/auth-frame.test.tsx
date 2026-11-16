import { renderToStaticMarkup } from "react-dom/server";
import { expect, it, vi } from "vitest";
vi.mock("@/components/theme-sync", () => ({
  ThemeToggle: () => <button aria-label="테마 전환" />,
}));
vi.mock("@/lib/use-api-action", () => ({
  useApiAction: () => async () => ({ ok: false }),
}));
import { AuthFrame } from "./auth-frame";
import { LoginScreen } from "./login-screen";
import { TwoFactorScreen } from "./two-factor-screen";
import { SetupScreen } from "./setup-screen";
import { SetupForm } from "@/app/setup/setup-form";
import { BrandLogo } from "@repo/ui";
import { APP_SEMVER } from "@/lib/version";
import styles from "./auth-screen.module.css";
it.each([false, true])(
  "keeps one heading, the full form and theme control (setup=%s)",
  (firstRun) => {
    const html = renderToStaticMarkup(
      <AuthFrame title="계정" subtitle="안내" firstRun={firstRun}>
        <form>
          <input aria-label="이메일" />
        </form>
      </AuthFrame>,
    );
    expect(html.match(/<h1[ >]/g)).toHaveLength(1);
    expect(html).toContain('aria-label="테마 전환"');
    expect(html).toContain('aria-label="이메일"');
    const footer = html.match(/<footer[^>]*>(.*?)<\/footer>/)![1]!;
    expect(footer).toContain(
      firstRun
        ? "관리자 계정은 최초 설정에서 한 번만 만들 수 있습니다."
        : "이 설치본에 등록된 계정으로 로그인하세요.",
    );
  },
);

it("keeps product identity in the header and a single account note in the footer", () => {
  const html = renderToStaticMarkup(
    <AuthFrame title="로그인" subtitle="안내">
      내용
    </AuthFrame>,
  );
  expect(html).not.toContain("<aside");
  const header = html.match(/<header[^>]*>(.*?)<\/header>/)![1]!;
  expect(header).not.toContain('aria-label="nginx-web-ui"');
  const heading = html.match(/<header[^>]*>(.*?)<\/header>/g)![1]!;
  expect(heading).toContain('aria-label="nginx-web-ui"');
  expect(heading.indexOf('aria-label="nginx-web-ui"')).toBeLessThan(
    heading.indexOf("<h1"),
  );
  expect(header).toContain('aria-label="테마 전환"');
  expect(
    html.match(/이 설치본에 등록된 계정으로 로그인하세요\./g),
  ).toHaveLength(1);
});

it("keeps login, setup and 2FA on the base one-column form layout", () => {
  const forms = [
    renderToStaticMarkup(<LoginScreen state="default" providers={[]} />),
    renderToStaticMarkup(<SetupForm />),
    renderToStaticMarkup(<TwoFactorScreen state="default" />),
  ];
  for (const html of forms) {
    expect(html.match(/<form[^>]*class="([^"]+)"/)![1]).toBe(styles.form);
    expect(html).not.toContain("data-wide");
  }
});

it("keeps the running version in the footer", () => {
  const html = renderToStaticMarkup(
    <AuthFrame title="로그인" subtitle="안내">
      내용
    </AuthFrame>,
  );
  const footer = html.match(/<footer[^>]*>(.*?)<\/footer>/)![1]!;
  expect(footer).toContain("nginx-web-ui");
  expect(footer).toContain(`v${APP_SEMVER}`);
});

it("shows the accessible mark without the wordmark on login, setup and 2FA", () => {
  for (const screen of [
    <LoginScreen key="login" state="default" providers={[]} />,
    <SetupScreen key="setup" />,
    <TwoFactorScreen key="2fa" state="default" />,
  ]) {
    const html = renderToStaticMarkup(screen);
    const header = html.match(/<header[^>]*>(.*?)<\/header>/)![1]!;
    expect(html).toContain('role="img"');
    expect(header).not.toContain('aria-label="nginx-web-ui"');
    const heading = html.match(/<header[^>]*>(.*?)<\/header>/g)![1]!;
    expect(heading).toContain('aria-label="nginx-web-ui"');
    expect(heading.indexOf('aria-label="nginx-web-ui"')).toBeLessThan(
      heading.indexOf("<h1"),
    );
    expect(header).not.toContain("<text");
    expect(html.match(/<footer[^>]*>(.*?)<\/footer>/)![1]).toContain(
      "nginx-web-ui",
    );
  }
});
it("keeps the full logo by default and the compact mark accessible", () => {
  expect(renderToStaticMarkup(<BrandLogo />)).toContain("<text");
  const compact = renderToStaticMarkup(<BrandLogo variant="compact" />);
  expect(compact).toContain('aria-label="nginx-web-ui"');
  expect(compact).not.toContain("<text");
});
