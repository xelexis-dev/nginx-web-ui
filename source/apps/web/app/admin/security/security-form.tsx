"use client";
import { Card, FormStack, SettingRows, SettingRow } from "@repo/ui";
import adminStyles from "@/app/admin/admin-pages.module.css";
import resourceStyles from "@/components/resource-layout.module.css";
import { FormErrors } from "@/components/form-errors";
import { useActionState, useState } from "react";
import { Banner, SegmentedControl, Button } from "@repo/ui";
import type { BasicFormState as PolicyState } from "@repo/contracts";
import { useApiAction } from "@/lib/use-api-action";

const INITIAL: PolicyState = { ok: false };

function Toggle({
  value,
  onChange,
  ariaLabel,
}: {
  value: boolean;
  onChange: (v: boolean) => void;
  ariaLabel: string;
}) {
  return (
    <SegmentedControl
      label={ariaLabel}
      value={value}
      onChange={onChange}
      options={[
        { value: false, label: "해제" },
        { value: true, label: "강제" },
      ]}
    />
  );
}

/**
 * 역할별 2FA 강제 정책 토글. 저장 시 변경된 역할군은 재로그인이 강제되어(서버가
 * sessionVersion 증가) 새 정책이 즉시 반영된다 — 본인이 포함되면 다음 이동에서
 * 로그인 화면으로 이동한다.
 */
export function SecurityForm({
  requireAdmin,
  requireUser,
  requireOauth,
}: {
  requireAdmin: boolean;
  requireUser: boolean;
  requireOauth: boolean;
}) {
  const [state, formAction, pending] = useActionState(
    useApiAction<PolicyState>({
      id: "admin.security.updateTwoFactorPolicy",
      args: [],
    }),
    INITIAL,
  );
  const [admin, setAdmin] = useState(requireAdmin);
  const [user, setUser] = useState(requireUser);
  const [oauth, setOauth] = useState(requireOauth);

  return (
    <form action={formAction} className={resourceStyles.form}>
      <FormErrors errors={state.fieldErrors} />
      <FormStack>
        <Card
          id="admin-security"
          title={<>보안 — 2FA 강제</>}
          errorCount={
            Object.values(state.fieldErrors ?? {}).filter(Boolean).length
          }
        >
          <div className={adminStyles.body}>
            <input type="hidden" name="require2faAdmin" value={String(admin)} />
            <input type="hidden" name="require2faUser" value={String(user)} />
            <input type="hidden" name="require2faOauth" value={String(oauth)} />

            <SettingRows>
              <SettingRow
                label="관리자 계정 2FA"
                wideDetail
                detail="admin 역할 계정에 2단계 인증을 강제합니다."
                action={
                  <Toggle
                    value={admin}
                    onChange={setAdmin}
                    ariaLabel="관리자 계정 2FA 강제"
                  />
                }
              />
              <SettingRow
                label="일반 계정 2FA"
                wideDetail
                detail="editor·viewer 역할 계정에 2단계 인증을 강제합니다."
                action={
                  <Toggle
                    value={user}
                    onChange={setUser}
                    ariaLabel="일반 계정 2FA 강제"
                  />
                }
              />
              <SettingRow
                label="OAuth 로그인 2FA"
                wideDetail
                detail="OAuth 로 로그인한 사용자에게도 이 앱의 2단계 인증을 요구합니다. 역할과 무관한 별개 축입니다 — 공급자가 이미 2단계 인증을 하는 경우가 많아, 켜면 이중이 될 수 있습니다."
                action={
                  <Toggle
                    value={oauth}
                    onChange={setOauth}
                    ariaLabel="OAuth 로그인 2FA 강제"
                  />
                }
              />
            </SettingRows>

            {state.ok ? (
              <Banner
                tone={"success"}
                title={<>정책이 저장되었습니다.</>}
                role="status"
              >
                <p>
                  강제를 변경한 역할의 사용자는 다음 요청에서 재로그인이
                  필요합니다. 본인 계정이 포함된 경우 곧 로그인 화면으로
                  이동합니다. 2FA 미등록 상태로 강제되면 로그인 후 등록 화면으로
                  안내됩니다.
                </p>
              </Banner>
            ) : null}
            {state.error ? (
              <Banner tone="error" role="alert" title={state.error} data-wide />
            ) : null}

            <footer className={adminStyles.foot}>
              <Button type="submit" variant="secondary" disabled={pending}>
                {pending ? "저장 중..." : "정책 저장"}
              </Button>
            </footer>
          </div>
        </Card>
      </FormStack>
    </form>
  );
}
