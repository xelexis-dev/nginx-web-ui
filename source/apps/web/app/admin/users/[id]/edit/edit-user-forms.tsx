"use client";
import { Card, FormStack, formFieldsClassName } from "@repo/ui";
import adminStyles from "@/app/admin/admin-pages.module.css";
import resourceStyles from "@/components/resource-layout.module.css";
import { FormErrors } from "@/components/form-errors";
import { useActionState, useRef, useState } from "react";
import Link from "next/link";
import {
  TextInput,
  buttonClassName,
  Button,
  Select,
  Field,
  Banner,
} from "@repo/ui";
import type { Role, BasicFormState as ActionState } from "@repo/contracts";
import { useApiAction } from "@/lib/use-api-action";

const INITIAL: ActionState = { ok: false };
const ROLE_OPTIONS: Role[] = ["admin", "editor", "viewer"];

/**
 * Edit page for one user: combines the 역할 변경 (role change) and
 * 비밀번호 재설정 (password reset) forms. Each is an independent
 * `useActionState` — submitting one never touches the other — and both
 * redirect back to the detail page (`/admin/users/[id]`) on success via their
 * server action (`updateUserRole` / `resetPassword`). Only field-validation
 * and the last-admin-demotion guard round-trip back here as errors.
 */
export function EditUserForms({
  id,
  email,
  currentRole,
  hasPassword,
}: {
  id: string;
  /** OAuth 로만 만들어진 계정은 이메일이 없다. */
  email: string | null;
  currentRole: Role;
  /** false 면 비밀번호 재설정 폼을 아예 렌더하지 않는다 — 그 계정에 비밀번호를
   *  만들어 주면 "로그인 문은 공급자 하나뿐"이라는 설계가 깨진다. 서버 액션에도
   *  같은 가드가 있다(UI 만 숨기면 폼을 직접 던져 우회된다). */
  hasPassword: boolean;
}) {
  const [roleState, roleAction, rolePending] = useActionState(
    useApiAction<ActionState>({ id: "admin.users.updateRole", args: [] }),
    INITIAL,
  );
  const [passwordState, passwordAction, passwordPending] = useActionState(
    useApiAction<ActionState>({ id: "admin.users.resetPassword", args: [] }),
    INITIAL,
  );
  const [mismatch, setMismatch] = useState<string | null>(null);
  const passwordRef = useRef<HTMLInputElement>(null);
  const confirmRef = useRef<HTMLInputElement>(null);

  return (
    <FormStack>
      {!hasPassword ? (
        <Banner
          tone="info"
          title={
            <>
              이 계정은 OAuth 로그인만 사용합니다 — 비밀번호가 없어 재설정할 수
              없습니다.
            </>
          }
        />
      ) : null}
      <form action={roleAction} className={resourceStyles.form}>
        <FormErrors errors={roleState.fieldErrors} />
        <FormStack>
          <Card
            id="admin-user-edit-0"
            title={<>역할 변경</>}
            errorCount={
              Object.values(roleState.fieldErrors ?? {}).filter(Boolean).length
            }
          >
            <div className={formFieldsClassName({ columns: 2 })}>
              <input type="hidden" name="id" value={id} />

              <Field
                label="역할"
                htmlFor="edit-user-role"
                error={roleState.fieldErrors?.role}
              >
                <Select
                  name="role"
                  defaultValue={currentRole}
                  options={ROLE_OPTIONS.map((role) => ({
                    value: role,
                    label: role,
                  }))}
                  aria-label="역할"
                />
              </Field>
              {roleState.error ? (
                <Banner
                  tone="error"
                  role="alert"
                  title={roleState.error}
                  data-wide
                />
              ) : null}
              <footer className={adminStyles.foot}>
                <Button
                  type="submit"
                  variant="secondary"
                  disabled={rolePending}
                >
                  {rolePending ? "저장 중..." : "역할 저장"}
                </Button>
              </footer>
            </div>
          </Card>
        </FormStack>
      </form>

      {hasPassword ? (
        <form
          action={passwordAction}
          className={resourceStyles.form}
          onSubmit={(event) => {
            if (passwordRef.current?.value !== confirmRef.current?.value) {
              event.preventDefault();
              setMismatch("비밀번호가 일치하지 않습니다.");
              return;
            }
            setMismatch(null);
          }}
        >
          <FormErrors
            errors={{
              ...passwordState.fieldErrors,
              ...(mismatch ? { confirmPassword: mismatch } : {}),
            }}
          />
          <FormStack>
            <Card
              id="admin-user-edit-1"
              title={<>{email ?? "이 사용자"} 비밀번호 재설정</>}
              errorCount={
                Object.values({
                  ...passwordState.fieldErrors,
                  ...(mismatch ? { confirmPassword: mismatch } : {}),
                }).filter(Boolean).length
              }
            >
              <div className={formFieldsClassName({ columns: 2 })}>
                <input type="hidden" name="id" value={id} />

                <Field
                  label="새 비밀번호 (8자 이상, 영문·숫자 포함)"
                  htmlFor="edit-user-password"
                  error={passwordState.fieldErrors?.password}
                >
                  <TextInput
                    ref={passwordRef}
                    name="password"
                    type="password"
                    autoComplete="new-password"
                    minLength={8}
                    required
                  />
                </Field>
                <Field
                  label="새 비밀번호 확인"
                  htmlFor="edit-user-password-confirm"
                  error={
                    [mismatch, passwordState.fieldErrors?.confirmPassword]
                      .filter(Boolean)
                      .join(" ") || undefined
                  }
                >
                  <TextInput
                    ref={confirmRef}
                    name="confirmPassword"
                    type="password"
                    autoComplete="new-password"
                    minLength={8}
                    required
                  />
                </Field>
                {passwordState.error ? (
                  <Banner
                    tone="error"
                    role="alert"
                    title={passwordState.error}
                    data-wide
                  />
                ) : null}
                <footer className={adminStyles.foot}>
                  <Button
                    type="submit"
                    variant="secondary"
                    disabled={passwordPending}
                  >
                    {passwordPending ? "저장 중..." : "비밀번호 재설정"}
                  </Button>
                </footer>
              </div>
            </Card>
          </FormStack>
        </form>
      ) : null}

      <div>
        <Link
          href={`/admin/users/${id}`}
          className={buttonClassName({ variant: "secondary" })}
        >
          취소
        </Link>
      </div>
    </FormStack>
  );
}
