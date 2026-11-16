"use client";
import { useActionState, useEffect, useId, useRef, useState } from "react";
import type { ActionRef, BasicFormState } from "@repo/contracts";
import { Banner, Button, Field, Modal, TextInput } from "@repo/ui";
import { useApiAction } from "@/lib/use-api-action";
import styles from "./passkeys.module.css";

export function RemovePasskeyButton({
  id,
  name,
}: {
  id: string;
  name: string;
}) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button variant="dangerQuiet" onClick={() => setOpen(true)}>
        삭제
      </Button>
      {open ? (
        <RemovePasskeyDialog
          action={{ id: "account.passkeys.remove", args: [] }}
          id={id}
          name={name}
          onClose={() => setOpen(false)}
        />
      ) : null}
    </>
  );
}
export function RemovePasskeyDialog({
  action: ref,
  id,
  name,
  userId,
  email,
  all = false,
  count = 0,
  isSelf = false,
  onClose,
}: {
  action: ActionRef;
  id?: string;
  name?: string;
  userId?: string;
  email?: string;
  all?: boolean;
  count?: number;
  isSelf?: boolean;
  onClose: () => void;
}) {
  const uid = useId();
  const password = useRef<HTMLInputElement>(null);
  const [state, action, pending] = useActionState(
    useApiAction<BasicFormState>(ref),
    { ok: false },
  );
  useEffect(() => {
    if (!pending) password.current?.focus();
  }, [pending, state]);
  return (
    <Modal
      open
      onClose={onClose}
      dismissible={!pending}
      icon="warning"
      title={
        all
          ? `패스키 ${count}개를 모두 삭제할까요?`
          : userId
            ? `${name ?? ""} 패스키를 삭제할까요?`
            : "이 패스키를 삭제할까요?"
      }
      description={
        <ul className={styles.effects}>
          {userId && !isSelf ? (
            <>
              <li>대상 계정: {email}</li>
              <li>
                사용자의 모든 기기에서 로그인이 종료됩니다. 삭제한 패스키는 다시
                등록해야 합니다.
              </li>
            </>
          ) : (
            <>
              <li>「{name}」 패스키로는 더 이상 로그인할 수 없습니다.</li>
              <li>
                이 계정의 다른 세션은 모두 로그아웃되고, 지금 세션은 유지됩니다.
              </li>
            </>
          )}
        </ul>
      }
      footer={
        <>
          <Button variant="secondary" onClick={onClose} disabled={pending}>
            취소
          </Button>
          <Button
            type="submit"
            form={uid}
            variant="danger"
            disabled={pending}
            aria-busy={pending || undefined}
          >
            {pending ? "삭제 중…" : all ? "전체 삭제" : "삭제"}
          </Button>
        </>
      }
    >
      <form id={uid} action={action} className={styles.sectionBody}>
        {id ? <input type="hidden" name="id" value={id} /> : null}
        {userId ? <input type="hidden" name="userId" value={userId} /> : null}
        {all ? <input type="hidden" name="all" value="1" /> : null}
        <Field
          label={userId ? "내 관리자 비밀번호" : "현재 비밀번호"}
          htmlFor={`${uid}-password`}
          required
          error={state.fieldErrors?.currentPassword}
          hint={
            userId
              ? "관리자 본인의 비밀번호입니다."
              : "삭제하려면 현재 비밀번호를 확인합니다."
          }
        >
          <TextInput
            ref={password}
            name="currentPassword"
            type="password"
            autoComplete="current-password"
            required
            disabled={pending}
            placeholder={userId ? "관리자 비밀번호 입력" : "현재 비밀번호 입력"}
          />
        </Field>
        {state.error ? <Banner tone="error" title={state.error} /> : null}
      </form>
    </Modal>
  );
}
