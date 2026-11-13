"use client";

import { useActionState, useId } from "react";
import Link from "next/link";
import type { ActionRef, BasicFormState } from "@repo/contracts";
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
import { PasskeyUnavailable } from "./passkey-list";
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
      <FormStack>
        <Banner
          tone="info"
          title={
            <>
              등록을 누른 뒤 기기의 안내에 따라 지문·얼굴 또는 PIN으로
              확인하세요. 등록하면 다른 기기의 로그인은 종료됩니다.
            </>
          }
        />
        <FormErrors errors={ceremony.result.fieldErrors} />
        <Card
          id={`${id}-register-card`}
          title="새 패스키"
          errorCount={
            Object.values(ceremony.result.fieldErrors ?? {}).filter(Boolean)
              .length
          }
        >
          <div className={formFieldsClassName({ columns: 2 })}>
            <Field
              label="이름"
              htmlFor={`${id}-name`}
              required
              error={ceremony.result.fieldErrors?.name}
              hint="나중에 알아보기 쉬운 이름을 입력하세요. 최대 64자."
            >
              <TextInput
                name="name"
                placeholder="예: 개인 노트북"
                maxLength={64}
                required
                disabled={waiting || disabled}
              />
            </Field>
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
            <div data-wide className={styles.readout}>
              <span className={styles.hint}>주소</span>
              <strong>{data.currentAddress ?? "—"}</strong>
            </div>

            {!data.availability.available ? (
              <div data-wide>
                <PasskeyUnavailable />
              </div>
            ) : !data.hasPassword ? (
              <p data-wide className={styles.hint}>
                외부 로그인 전용 계정은 패스키를 등록할 수 없습니다.
              </p>
            ) : data.passkeys.length >= 10 ? (
              <div data-wide>
                <Banner
                  tone="warning"
                  title="패스키는 최대 10개까지 등록할 수 있습니다."
                />
              </div>
            ) : !ceremony.supported ? (
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
          note={<>등록 가능: {Math.max(0, 10 - data.passkeys.length)}개</>}
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
            <Icon name={waiting ? "pending" : "tokens"} />
            {waiting ? "기기 확인 중" : "패스키 등록"}
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
      <FormStack>
        <FormErrors errors={state.fieldErrors} />
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

export function RemovePasskeyForm({
  action: ref,
  id,
  userId,
  email,
  all,
  canRemove,
  isSelf = false,
  onCancel,
}: {
  action: ActionRef;
  id?: string;
  userId?: string;
  email?: string;
  all?: boolean;
  canRemove: boolean;
  isSelf?: boolean;
  onCancel?: () => void;
}) {
  const uid = useId();
  const [state, action, pending] = useActionState(
    useApiAction<BasicFormState>(ref),
    { ok: false },
  );
  return (
    <form action={action}>
      <FormErrors errors={state.fieldErrors} />
      {id ? <input type="hidden" name="id" value={id} /> : null}
      {userId ? <input type="hidden" name="userId" value={userId} /> : null}
      {all ? <input type="hidden" name="all" value="1" /> : null}
      <div className={formFieldsClassName({ columns: 2 })}>
        <div>
          <p>
            {userId
              ? `대상 계정: ${email}`
              : "이 패스키로 더 이상 로그인할 수 없습니다."}
          </p>
          <p className={styles.hint}>
            {userId && !isSelf
              ? "사용자의 모든 기기에서 로그인이 종료됩니다. 삭제한 패스키는 다시 등록해야 합니다."
              : "다른 기기의 로그인도 종료됩니다. 이 기기의 로그인은 유지됩니다."}
          </p>
        </div>
        <Field
          label={userId ? "내 관리자 비밀번호" : "현재 비밀번호"}
          htmlFor={`${uid}-password`}
          required
          error={state.fieldErrors?.currentPassword}
        >
          <TextInput
            name="currentPassword"
            type="password"
            autoComplete="current-password"
            placeholder={userId ? "관리자 비밀번호 입력" : "현재 비밀번호 입력"}
            required
            disabled={!canRemove || pending}
          />
        </Field>
      </div>
      {!canRemove ? (
        <p className={styles.hint}>
          {userId
            ? "외부 로그인 전용 관리자 계정은 패스키를 삭제할 수 없습니다. 비밀번호를 사용하는 관리자에게 요청하세요."
            : "이 계정에서는 현재 비밀번호로 본인 확인을 할 수 없어 패스키를 삭제할 수 없습니다."}
        </p>
      ) : null}
      {state.error ? (
        <Banner tone="error" role="alert" title={state.error} />
      ) : null}
      <div className={styles.actions}>
        {!userId ? (
          <span className={styles.hint}>
            다른 패스키나 비밀번호로 다시 로그인할 수 있습니다.
          </span>
        ) : null}
        {onCancel ? (
          <Button
            type="button"
            variant="secondary"
            onClick={onCancel}
            disabled={pending}
          >
            취소
          </Button>
        ) : null}
        <Button
          type="submit"
          variant={userId ? "danger" : "dangerQuiet"}
          disabled={!canRemove || pending}
        >
          {all ? "패스키 전체 삭제" : "패스키 삭제"}
        </Button>
      </div>
    </form>
  );
}
