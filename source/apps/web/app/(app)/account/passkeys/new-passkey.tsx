import Link from "next/link";
import { Banner, Icon, PageHeader, buttonClassName } from "@repo/ui";
import { PasskeyAvailabilityNotice } from "@/components/passkeys/passkey-availability-notice";
import { type PasskeyListView } from "@/lib/passkey-api";
import { RegisterPasskeyForm } from "@/components/passkeys/passkey-forms";
import layout from "@/components/page-layout.module.css";

export function NewPasskeyView({
  data,
  role = "viewer",
}: {
  data: PasskeyListView;
  role?: string;
}) {
  const blocked =
    !data.hasPassword ||
    data.passkeys.length >= 10 ||
    !data.availability.available;
  return (
    <section className={layout.page}>
      <PageHeader
        title="패스키 등록"
        description={
          !blocked
            ? "이름과 현재 비밀번호를 입력하고 만들면, 브라우저가 패스키를 저장할 기기나 비밀번호 관리자를 묻습니다. 등록하면 이 계정의 다른 세션은 로그아웃됩니다."
            : undefined
        }
      />
      {blocked ? (
        <div className={layout.page}>
          {!data.hasPassword ? (
            <Banner
              tone="info"
              title="외부 로그인만 쓰는 계정은 패스키를 등록할 수 없습니다."
            />
          ) : data.passkeys.length >= 10 ? (
            <Banner
              tone="info"
              title="패스키는 최대 10개까지 등록할 수 있습니다. 새로 등록하려면 사용하지 않는 패스키를 삭제하세요."
            />
          ) : (
            <PasskeyAvailabilityNotice
              reason={data.availability.reason}
              role={role}
            />
          )}
          <div>
            <Link
              href="/account"
              className={buttonClassName({ variant: "secondary" })}
            >
              <Icon name="arrowLeft" />
              계정으로 돌아가기
            </Link>
          </div>
        </div>
      ) : (
        <RegisterPasskeyForm data={data} />
      )}
    </section>
  );
}
