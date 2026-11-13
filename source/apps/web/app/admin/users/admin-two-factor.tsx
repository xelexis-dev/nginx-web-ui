"use client";

import { useActionState, useEffect, useId, useRef, useState } from "react";
import Link from "next/link";
import {
  Banner,
  Button,
  Facts,
  Field,
  Modal,
  StatusBadge,
  Surface,
  TextInput,
  linkClassName,
} from "@repo/ui";
import type { BasicFormState } from "@repo/contracts";
import type { AdminUserView } from "@/lib/admin-api";
import { useApiAction } from "@/lib/use-api-action";

export function AdminTwoFactor({
  user,
  isSelf,
  canReset,
  passkeyCount,
  reset = false,
}: {
  user: AdminUserView;
  isSelf: boolean;
  canReset: boolean;
  passkeyCount: number;
  reset?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const notice = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (reset) {
      notice.current?.focus();
    }
  }, [reset]);
  return (
    <Surface
      title="2단계 인증"
      actions={
        user.totpEnabled && !isSelf && canReset ? (
          <Button variant="dangerQuiet" size="sm" onClick={() => setOpen(true)}>
            2FA 초기화
          </Button>
        ) : undefined
      }
    >
      {reset ? (
        <div ref={notice} tabIndex={-1}>
          <Banner
            tone="success"
            title="2FA를 초기화했습니다. 인증 앱 등록, 백업 코드와 패스키를 삭제하고 사용자의 로그인을 종료했습니다."
          />
        </div>
      ) : null}
      <Facts
        items={[
          {
            term: "인증 앱(OTP)",
            value: (
              <StatusBadge tone={user.totpEnabled ? "success" : "neutral"}>
                {user.totpEnabled ? "사용 중" : "미설정"}
              </StatusBadge>
            ),
          },
        ]}
      />
      {isSelf ? (
        <p>
          본인 계정의 2FA는 계정 화면에서 관리합니다.{" "}
          <Link href="/account/2fa" className={linkClassName()}>
            계정 화면에서 관리
          </Link>
        </p>
      ) : !canReset ? (
        <Banner
          tone="info"
          title="외부 로그인 전용 관리자 계정은 2FA를 초기화할 수 없습니다."
        >
          비밀번호를 사용하는 관리자에게 요청하세요.
        </Banner>
      ) : !user.totpEnabled ? (
        <p>설정된 2FA가 없어 초기화할 항목이 없습니다.</p>
      ) : null}
      {open && user.totpEnabled && !isSelf && canReset ? (
        <ResetDialog
          user={user}
          passkeyCount={passkeyCount}
          onClose={() => setOpen(false)}
        />
      ) : null}
    </Surface>
  );
}

export function ResetDialog({
  user,
  passkeyCount,
  onClose,
}: {
  user: AdminUserView;
  passkeyCount: number;
  onClose: () => void;
}) {
  const id = useId();
  const [state, action, pending] = useActionState(
    useApiAction<BasicFormState>({
      id: "admin.users.resetTwoFactor",
      args: [],
    }),
    { ok: false },
  );
  const password = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (!pending && (state.error || state.fieldErrors?.currentPassword))
      password.current?.focus();
  }, [pending, state]);
  return (
    <Modal
      open
      onClose={onClose}
      title={`${user.email ?? "이메일 없음"}의 2FA를 초기화하시겠습니까?`}
      icon="danger"
      description={
        <>
          <p>
            인증 앱 등록과 백업 코드
            {passkeyCount ? `, 패스키 ${passkeyCount}개` : ""}를 모두 삭제하고
            이 사용자의 모든 로그인을 종료합니다. 되돌릴 수 없습니다.
          </p>
          <p>
            사용자는 다시 로그인한 뒤 2FA를 새로 설정합니다. 2FA가 필수이면
            설정을 마쳐야 다른 화면을 쓸 수 있습니다.
          </p>
        </>
      }
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>
            취소
          </Button>
          <Button
            type="submit"
            form={id}
            variant="danger"
            disabled={pending}
            aria-busy={pending || undefined}
          >
            {pending ? "처리 중..." : "2FA 초기화"}
          </Button>
        </>
      }
    >
      <form id={id} action={action}>
        <input type="hidden" name="id" value={user.id} />
        <Field
          label="현재 비밀번호"
          htmlFor={`${id}-password`}
          required
          hint="관리자 본인의 비밀번호입니다."
          error={state.fieldErrors?.currentPassword}
        >
          <TextInput
            ref={password}
            name="currentPassword"
            type="password"
            autoComplete="current-password"
            required
            disabled={pending}
          />
        </Field>
        {state.error ? (
          <Banner tone="error" role="alert" title={state.error} />
        ) : null}
      </form>
    </Modal>
  );
}
