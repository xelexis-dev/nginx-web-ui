"use client";
import { Card, FormStack } from "@repo/ui";
import adminStyles from "@/app/admin/admin-pages.module.css";
import resourceStyles from "@/components/resource-layout.module.css";
import { FormErrors } from "@/components/form-errors";
import { useActionState, useState } from "react";
import {
  Banner,
  TextInput,
  Button,
  Field,
  formFieldsClassName,
} from "@repo/ui";
import { useApiAction } from "@/lib/use-api-action";
import type { GeneralSettingsState } from "./form-types";

const INITIAL: GeneralSettingsState = { ok: false };

/**
 * 알림 링크를 절대 URL 로 만들기 위한 앱 주소.
 *
 * 배경 스윕에는 HTTP 요청이 없어 호스트를 스스로 알 수 없고, Discord embed 와 메일
 * 본문은 상대 URL 을 쓸 수 없다 — 그래서 설정으로 받는다. 비워두면 링크가 상대경로로
 * 나가고 Discord·메일은 링크를 붙이지 않는다(알림 자체는 정상 발송된다).
 */
export function BaseUrlForm({ defaultValue }: { defaultValue: string }) {
  const [state, action, pending] = useActionState(
    useApiAction<GeneralSettingsState>({
      id: "admin.settings.updatePublicBaseUrl",
      args: [],
    }),
    INITIAL,
  );
  const [value, setValue] = useState(defaultValue);

  return (
    <form className={resourceStyles.form} action={action}>
      <FormStack>
        <FormErrors errors={state.fieldErrors} />
        <Card
          id="admin-base-url"
          title={<>일반</>}
          errorCount={
            Object.values(state.fieldErrors ?? {}).filter(Boolean).length
          }
        >
          <div className={adminStyles.body}>
            <div className={formFieldsClassName()}>
              <Field
                label="앱 주소 (선택)"
                htmlFor="public-base-url"
                hint="알림 속 링크를 누를 수 있게 만듭니다. 비우면 링크가 동작하지 않습니다."
                error={state.fieldErrors?.publicBaseUrl}
                wide
              >
                <TextInput
                  name="publicBaseUrl"
                  value={value}
                  onChange={(e) => setValue(e.target.value)}
                  placeholder="https://nginx.example.com"
                />
              </Field>
            </div>

            {state.error ? (
              <Banner
                tone={"error"}
                title={<>저장 실패 — {state.error}</>}
                role="alert"
              ></Banner>
            ) : null}
            {state.ok && state.message ? (
              <Banner
                tone={"success"}
                title={<>{state.message}</>}
                role="status"
              ></Banner>
            ) : null}

            <footer className={adminStyles.foot}>
              <Button type="submit" variant="secondary" disabled={pending}>
                {pending ? "저장 중..." : "저장"}
              </Button>
            </footer>
          </div>
        </Card>
      </FormStack>
    </form>
  );
}
