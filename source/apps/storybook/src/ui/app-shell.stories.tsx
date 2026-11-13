import {
  AppShell,
  Button,
  PageHeader,
  Surface,
  type AppShellNavGroup,
  type AppShellProps,
} from "@repo/ui";
import { useEffect, useState } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { NAV_CATEGORIES } from "@/app/(app)/nav-links";
import { HeaderActions } from "@/components/operations/header-actions";
import { ProfileMenu } from "@/components/profile-menu";
import pageLayout from "@/components/page-layout.module.css";
import { APP_SEMVER, FULL_SHA, SHA_LABEL } from "@/lib/version";
const nav: AppShellNavGroup[] = NAV_CATEGORIES.map((group) => ({
  ...group,
  items: group.items.map((item) => ({
    ...item,
    icon: item.icon,
    match: item.href === "/" ? "exact" : "prefix",
  })),
}));
function Interactive(props: AppShellProps) {
  const [path, setPath] = useState(props.currentPath);
  useEffect(() => setPath(props.currentPath), [props.currentPath]);
  const label =
    path === "/"
      ? "개요"
      : (nav.flatMap((g) => g.items).find((i) => i.href === path)?.label ??
        "리버스 프록시");
  return (
    <AppShell
      {...props}
      currentPath={path}
      renderLink={({ onClick, ...link }) => (
        <a
          {...link}
          onClick={(event) => {
            event.preventDefault();
            setPath(link.href);
            onClick?.();
          }}
        />
      )}
    >
      <div className={pageLayout.page}>
        <PageHeader title={label} description="공통 내비게이션과 대상 표시" />
        {props.children}
      </div>
    </AppShell>
  );
}
const meta = {
  title: "UI/AppShell",
  component: AppShell,
  parameters: { layout: "fullscreen" },
  args: {
    nav,
    currentPath: "/",
    topbarContext: <span>로컬 서버</span>,
    topbarActions: (
      <HeaderActions
        canApply
        pendingCount={2}
        onApply={() => {}}
        profile={
          <ProfileMenu
            email="admin@example.test"
            role="admin"
            version={APP_SEMVER}
            sha={SHA_LABEL}
            fullSha={FULL_SHA}
          />
        }
      />
    ),
    version: APP_SEMVER,
    children: (
      <Surface title="운영 화면">
        <p>페이지는 가운데 1200px 안에서 폭을 채웁니다.</p>
        <Button>본문 동작</Button>
      </Surface>
    ),
  },
  render: (args) => <Interactive {...args} />,
} satisfies Meta<typeof AppShell>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = {};
export const Dark: Story = { globals: { theme: "dark" } };
export const SidebarScroll: Story = {
  parameters: { viewport: { defaultViewport: "mobile1" } },
};
export const Mobile390: Story = {
  globals: { viewport: { value: "p390", isRotated: false } },
};
export const LongTarget: Story = {
  args: {
    topbarContext: (
      <span
        style={{
          minWidth: 0,
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
        }}
        title="productiongatewaywithoutwordbreaks"
      >
        productiongatewaywithoutwordbreaks
      </span>
    ),
  },
};
