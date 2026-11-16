"use client";

import { useActionState, useId } from "react";
import Link from "next/link";
import type { BasicFormState } from "@repo/contracts";
import {
  Card,
  FormStack,
  FormBar,
  Banner,
  Button,
  Field,
  Icon,
  TextInput,
  buttonClassName,
  formFieldsClassName,
} from "@repo/ui";
import { useApiAction } from "@/lib/use-api-action";
import { usePasskeyCeremony } from "@/lib/passkey-client";
import type { PasskeyListView, PasskeyRowView } from "@/lib/passkey-api";
import { FormErrors } from "@/components/form-errors";
import styles from "./passkeys.module.css";

export function RegisterPasskeyForm({ data }: { data: PasskeyListView }) {
  const id = useId();
  const ceremony = usePasskeyCeremony("registration");
  const waiting = ceremony.phase === "waiting";
  const disabled =
    !data.availability.available ||
    !data.hasPassword ||
    data.passkeys.length >= 10 ||
    !ceremony.supported;
  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        if (!disabled) void ceremony.submit(new FormData(event.currentTarget));
      }}
    >
      <FormErrors errors={ceremony.result.fieldErrors} />
      <FormStack>
        <Card
          id={`${id}-register-card`}
          title="패스키"
          description="어느 기기의 패스키인지 알아볼 수 있는 이름을 붙입니다."
          errorCount={ceremony.result.fieldErrors?.name ? 1 : 0}
        >
          <div className={formFieldsClassName({ columns: 2 })}>
            <Field
              label="이름"
              htmlFor={`${id}-name`}
              required
              error={ceremony.result.fieldErrors?.name}
              hint="나중에 바꿀 수 있습니다. 최대 64자."
            >
              <TextInput
                name="name"
                placeholder="예: 업무용 노트북"
                maxLength={64}
                required
                disabled={waiting || disabled}
              />
            </Field>
            <Field
              label="등록 주소"
              htmlFor={`${id}-address`}
              hint="이 패스키는 이 주소로 접속했을 때만 로그인에 쓸 수 있습니다."
            >
              <TextInput value={data.currentAddress ?? "—"} readOnly />
            </Field>
          </div>
        </Card>
        <Card
          id={`${id}-confirmation-card`}
          errorCount={ceremony.result.fieldErrors?.currentPassword ? 1 : 0}
          title="본인 확인"
          description="패스키를 등록하려면 현재 비밀번호를 확인합니다."
        >
          <div className={formFieldsClassName({ columns: 2 })}>
            <Field
              label="현재 비밀번호"
              htmlFor={`${id}-password`}
              required
              error={ceremony.result.fieldErrors?.currentPassword}
              hint="계정 소유자 확인을 위해 필요합니다."
            >
              <TextInput
                name="currentPassword"
                type="password"
                autoComplete="current-password"
                placeholder="현재 비밀번호 입력"
                required
                disabled={waiting || disabled}
              />
            </Field>
            {!ceremony.supported ? (
              <p data-wide className={styles.hint}>
                이 브라우저에서는 패스키를 등록할 수 없습니다. 지원하는
                브라우저로 다시 접속하세요.
              </p>
            ) : null}
            {waiting ? (
              <div data-wide>
                <Banner tone="info" title="기기에서 확인해 주세요.">
                  브라우저나 기기에 나타난 안내에서 패스키 등록을 완료하세요.
                </Banner>
              </div>
            ) : null}
            {ceremony.phase === "cancelled" ? (
              <p data-wide className={styles.hint} role="status">
                패스키 등록을 취소했습니다. 다시 시도할 수 있습니다.
              </p>
            ) : null}
            {ceremony.phase === "duplicate" ? (
              <div data-wide>
                <Banner
                  tone="warning"
                  title="이 기기에는 이미 등록된 패스키가 있습니다"
                />
              </div>
            ) : null}
            {ceremony.result.error ? (
              <div data-wide>
                <Banner
                  tone="error"
                  role="alert"
                  title={ceremony.result.error}
                />
              </div>
            ) : ceremony.phase === "failed" ? (
              <div data-wide>
                <Banner
                  tone="error"
                  role="alert"
                  title="패스키를 등록하지 못했습니다. 다시 시도해 주세요."
                />
              </div>
            ) : null}
          </div>
        </Card>
        <FormBar
          note={
            <>
              등록 주소: <strong>{data.currentAddress}</strong>
            </>
          }
        >
          {waiting ? (
            <Button type="button" variant="secondary" onClick={ceremony.cancel}>
              취소
            </Button>
          ) : (
            <Link
              href="/account"
              className={buttonClassName({ variant: "secondary" })}
            >
              취소
            </Link>
          )}
          <Button type="submit" disabled={waiting || disabled}>
            <Icon name={waiting ? "pending" : "fingerprint"} />
            {waiting ? "기기 확인 중" : "패스키 만들기"}
          </Button>
        </FormBar>
      </FormStack>
    </form>
  );
}

export function RenamePasskeyForm({ row }: { row: PasskeyRowView }) {
  const id = useId();
  const [state, action, pending] = useActionState(
    useApiAction<BasicFormState>({ id: "account.passkeys.rename", args: [] }),
    { ok: false },
  );
  return (
    <form action={action}>
      <input type="hidden" name="id" value={row.id} />
      <FormErrors errors={state.fieldErrors} />
      <FormStack>
        <Card
          id={`${id}-rename-card`}
          title="표시 이름"
          errorCount={
            Object.values(state.fieldErrors ?? {}).filter(Boolean).length
          }
        >
          <div className={formFieldsClassName({ columns: 1 })}>
            <Field
              label="이름"
              htmlFor={`${id}-name`}
              required
              error={state.fieldErrors?.name}
              hint="최대 64자. 이름을 변경해도 로그인 방법은 바뀌지 않습니다."
            >
              <TextInput
                name="name"
                defaultValue={row.name}
                maxLength={64}
                required
              />
            </Field>
            {state.error ? (
              <Banner tone="error" role="alert" title={state.error} />
            ) : null}
          </div>
        </Card>
        <FormBar>
          <Link
            href={`/account/passkeys/${encodeURIComponent(row.id)}`}
            className={buttonClassName({ variant: "secondary" })}
          >
            취소
          </Link>
          <Button type="submit" disabled={pending}>
            이름 저장
          </Button>
        </FormBar>
      </FormStack>
    </form>
  );
}
