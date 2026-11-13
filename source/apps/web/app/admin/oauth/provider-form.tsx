"use client";
import { Card, FormStack, FormBar } from "@repo/ui";
import adminStyles from "@/app/admin/admin-pages.module.css";
import resourceStyles from "@/components/resource-layout.module.css";
import { FormErrors } from "@/components/form-errors";
import { useActionState, useState } from "react";
import Link from "next/link";
import {
  TextInput,
  buttonClassName,
  Button,
  Select,
  Field,
  Banner,
  Facts,
  formFieldsClassName,
} from "@repo/ui";
import { useApiAction } from "@/lib/use-api-action";
import type { ProviderFormState } from "./form-types";
import styles from "./oauth.module.css";

const INITIAL: ProviderFormState = { ok: false };

/**
 * 수정 폼이 비밀키 원문을 되받지 않으므로, 저장할 때 이 표식을 보내 "그대로 둬"를
 * 뜻한다. actions.ts 가 같은 값을 들고 있다 — `"use server"` 파일은 async 함수
 * 외에는 내보낼 수 없어서 상수를 공유할 수 없다(webhook/SMTP 폼과 같은 사정).
 */
const KEEP_SECRET = "__keep__";

export interface ProviderDefaults {
  id: string;
  name: string;
  issuer: string;
  clientId: string;
  scope: string;
  enabled: boolean;
  sortOrder: number;
}

export function ProviderForm({
  mode,
  defaults,
  callbackUrl,
}: {
  mode: "create" | "edit";
  defaults?: ProviderDefaults;
  /** 운영자가 공급자 쪽에 등록해야 하는 주소. 수정 화면에서만 보여준다. */
  callbackUrl?: string;
}) {
  const [state, action, pending] = useActionState(
    useApiAction<ProviderFormState>({
      id: mode === "create" ? "admin.oauth.create" : "admin.oauth.update",
      args: [],
    }),
    INITIAL,
  );
  const isEdit = mode === "edit";
  const [values, setValues] = useState({
    id: defaults?.id ?? "",
    name: defaults?.name ?? "",
    issuer: defaults?.issuer ?? "",
    clientId: defaults?.clientId ?? "",
    scope: defaults?.scope ?? "openid email profile",
    sortOrder: String(defaults?.sortOrder ?? 0),
    enabled: String(defaults?.enabled ?? true),
  });
  function change(key: keyof typeof values, value: string) {
    setValues((previous) => ({ ...previous, [key]: value }));
  }

  return (
    <form action={action} className={resourceStyles.form}>
      <FormStack>
        <FormErrors
          errors={state.fieldErrors}
          targets={{
            clientSecret: "provider-client-secret",
            clientId: "provider-client-id",
            sortOrder: "provider-sort",
          }}
        />
        <Card
          id="admin-provider"
          title={isEdit ? "공급자 수정" : "공급자 추가"}
          errorCount={
            Object.values(state.fieldErrors ?? {}).filter(Boolean).length
          }
        >
          <div className={adminStyles.body}>
            {isEdit ? (
              <input type="hidden" name="id" value={defaults?.id ?? ""} />
            ) : null}

            <div className={formFieldsClassName()}>
              <Field
                label="공급자 ID"
                htmlFor="provider-id"
                hint={
                  isEdit ? (
                    "ID 는 콜백 주소에 그대로 들어가므로 만든 뒤에는 바꿀 수 없습니다."
                  ) : (
                    <>
                      영소문자·숫자·하이픈. 콜백 주소{" "}
                      <code className={styles.hintCode}>
                        /api/auth/callback/&lt;ID&gt;
                      </code>{" "}
                      에 그대로 들어가며 나중에 바꿀 수 없습니다.
                    </>
                  )
                }
                error={state.fieldErrors?.id}
              >
                {isEdit ? (
                  <TextInput value={defaults?.id ?? ""} readOnly disabled />
                ) : (
                  <TextInput
                    name="id"
                    value={values.id}
                    onChange={(event) => change("id", event.target.value)}
                    placeholder="google"
                    autoComplete="off"
                    required
                  />
                )}
              </Field>

              <Field
                label="표시 이름"
                htmlFor="provider-name"
                hint="로그인 화면 버튼에 「○○ 로 로그인」으로 적힙니다."
                error={state.fieldErrors?.name}
              >
                <TextInput
                  name="name"
                  value={values.name}
                  onChange={(event) => change("name", event.target.value)}
                  placeholder="Google"
                  required
                />
              </Field>

              <Field
                label="로그인 서버 주소 (issuer)"
                htmlFor="provider-issuer"
                hint={
                  <>
                    이 주소의{" "}
                    <code className={styles.hintCode}>
                      /.well-known/openid-configuration
                    </code>{" "}
                    을 읽어 나머지 설정을 자동으로 맞춥니다. https 만 됩니다.
                  </>
                }
                error={state.fieldErrors?.issuer}
                wide
              >
                <TextInput
                  name="issuer"
                  value={values.issuer}
                  onChange={(event) => change("issuer", event.target.value)}
                  placeholder="https://accounts.google.com"
                  required
                />
              </Field>

              <Field
                label="클라이언트 ID"
                htmlFor="provider-client-id"
                error={state.fieldErrors?.clientId}
              >
                <TextInput
                  name="clientId"
                  value={values.clientId}
                  onChange={(event) => change("clientId", event.target.value)}
                  autoComplete="off"
                  required
                />
              </Field>

              <Field
                label="클라이언트 비밀키"
                htmlFor="provider-client-secret"
                hint={
                  isEdit
                    ? "그대로 두면 저장된 비밀키를 유지합니다. 바꾸려면 지우고 새 값을 넣으세요."
                    : "저장할 때 암호로 잠급니다. 이후 화면에 다시 보여주지 않습니다."
                }
                error={state.fieldErrors?.clientSecret}
              >
                <TextInput
                  name="clientSecret"
                  type="password"
                  defaultValue={isEdit ? KEEP_SECRET : ""}
                  autoComplete="new-password"
                />
              </Field>

              <Field
                label="scope"
                htmlFor="provider-scope"
                hint="비워 두면 기본값을 씁니다. openid 는 자동으로 포함됩니다."
              >
                <TextInput
                  name="scope"
                  value={values.scope}
                  onChange={(event) => change("scope", event.target.value)}
                  autoComplete="off"
                />
              </Field>

              <Field
                label="정렬 순서"
                htmlFor="provider-sort"
                hint="작을수록 로그인 화면에서 위에 옵니다."
                error={state.fieldErrors?.sortOrder}
              >
                <TextInput
                  name="sortOrder"
                  type="number"
                  value={values.sortOrder}
                  onChange={(event) => change("sortOrder", event.target.value)}
                />
              </Field>

              <Field label="사용" htmlFor="provider-enabled">
                <Select
                  name="enabled"
                  value={values.enabled}
                  onChange={(value) => change("enabled", value)}
                  options={[
                    { value: "true", label: "사용" },
                    { value: "false", label: "사용 안 함 (설정만 보관)" },
                  ]}
                  aria-label="사용 여부"
                />
              </Field>

              {callbackUrl ? (
                <div data-wide>
                  <Facts
                    items={[
                      {
                        term: "공급자 쪽에 등록할 콜백 주소",
                        value: (
                          <>
                            <code className={styles.callback}>
                              {callbackUrl}
                            </code>
                            <p className={styles.hint}>
                              이 주소를 공급자의 「승인된 리디렉션 URI」에
                              그대로 넣으세요.
                            </p>
                          </>
                        ),
                      },
                    ]}
                  />
                </div>
              ) : null}
            </div>

            {state.error ? (
              <Banner
                tone="error"
                role="alert"
                title={state.error}
                style={{ gridColumn: "1 / -1" }}
              />
            ) : null}
          </div>
        </Card>
        <FormBar>
          <Link
            href="/admin/oauth"
            className={buttonClassName({ variant: "secondary" })}
          >
            취소
          </Link>
          <Button type="submit" disabled={pending}>
            {pending ? "저장 중..." : isEdit ? "저장" : "공급자 추가"}
          </Button>
        </FormBar>
      </FormStack>
    </form>
  );
}
