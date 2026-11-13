import { TemplatePreview } from "./preview";
// Template: a settings page (Storybook only). The contract a settings screen follows, built from
// @repo/ui alone: header, the saved notice when there is one, then one card per group. A group of
// choices is a list of settings (name and note left, control right); a group of values is a
// two-column FormSection. One save for the page.
import {
  Banner,
  Button,
  PageHeader,
  SegmentedControl,
  Surface,
} from "@repo/ui";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { useState } from "react";
import { productFrame, productViewports } from "../product/frame";
import styles from "./templates.module.css";

function SettingsTemplate({ saved }: { saved: boolean }) {
  const [admin, setAdmin] = useState(true);
  const [user, setUser] = useState(false);
  const [oauth, setOauth] = useState(true);
  return (
    <TemplatePreview className={styles.page}>
      <PageHeader
        title="보안"
        description="로그인과 세션에 관한 서버 전체 설정입니다."
      />
      <form
        className={styles.stack}
        onSubmit={(event) => event.preventDefault()}
      >
        {saved ? (
          <Banner tone="success" title="저장했습니다.">
            바뀐 역할의 사용자는 다음 이동에서 다시 로그인합니다.
          </Banner>
        ) : null}
        <Surface
          title="2단계 인증"
          description="역할마다 2단계 인증을 강제할 수 있습니다."
        >
          <ul className={styles.settingList}>
            <li className={styles.setting}>
              <div className={styles.settingText}>
                <p className={styles.settingName}>관리자 계정</p>
                <p className={styles.settingNote}>
                  admin 역할 계정에 2단계 인증을 강제합니다.
                </p>
              </div>
              <SegmentedControl
                label="관리자 계정 2FA 강제"
                options={[
                  { value: false, label: "해제" },
                  { value: true, label: "강제" },
                ]}
                value={admin}
                onChange={setAdmin}
              />
            </li>
            <li className={styles.setting}>
              <div className={styles.settingText}>
                <p className={styles.settingName}>일반 계정</p>
                <p className={styles.settingNote}>
                  editor, viewer 역할 계정에 강제합니다.
                </p>
              </div>
              <SegmentedControl
                label="일반 계정 2FA 강제"
                options={[
                  { value: false, label: "해제" },
                  { value: true, label: "강제" },
                ]}
                value={user}
                onChange={setUser}
              />
            </li>
            <li className={styles.setting}>
              <div className={styles.settingText}>
                <p className={styles.settingName}>OAuth 로그인 2FA</p>
                <p className={styles.settingNote}>
                  OAuth로 로그인한 사용자에게도 이 앱의 2단계 인증을 요구합니다.
                </p>
              </div>
              <SegmentedControl
                label="OAuth 로그인 2FA 강제"
                options={[
                  { value: false, label: "해제" },
                  { value: true, label: "강제" },
                ]}
                value={oauth}
                onChange={setOauth}
              />
            </li>
          </ul>
        </Surface>
        <div className={styles.actions}>
          <Button type="submit">저장</Button>
        </div>
      </form>
    </TemplatePreview>
  );
}

const meta = {
  id: "templates-settings",
  title: "Templates/설정",
  component: SettingsTemplate,
  tags: ["!autodocs"],
  parameters: { layout: "fullscreen", viewport: { options: productViewports } },
  decorators: [
    productFrame(() => ({
      path: "/admin/security",
      label: "보안",
      role: "admin",
    })),
  ],
  args: { saved: false },
} satisfies Meta<typeof SettingsTemplate>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = { name: "설정" };
export const Saved: Story = { name: "저장함", args: { saved: true } };
export const Narrow: Story = {
  name: "설정 · 390",
  globals: { viewport: { value: "p390", isRotated: false } },
};

export const Dark: Story = { globals: { theme: "dark" } };
