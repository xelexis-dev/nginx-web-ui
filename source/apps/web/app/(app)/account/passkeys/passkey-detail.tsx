import Link from "next/link";
import {
  Banner,
  Facts,
  Icon,
  PageHeader,
  Surface,
  StatusBadge,
  buttonClassName,
  linkClassName,
} from "@repo/ui";
import { passkeyRowView, type PasskeyDetail } from "@/lib/passkey-api";
import { RemovePasskeyButton } from "@/components/passkeys/remove-passkey-dialog";
import layout from "@/components/page-layout.module.css";

export function PasskeyDetailView({
  data,
  query = {},
}: {
  data: PasskeyDetail;
  query?: { added?: string; renamed?: string };
}) {
  const row = passkeyRowView(data.passkey);
  const id = row.id;
  return (
    <section className={layout.page}>
      <PageHeader
        title={row.name}
        status={
          <StatusBadge tone={row.unavailableHere ? "neutral" : "success"}>
            {row.unavailableHere
              ? "이 주소에서 사용 불가"
              : "이 주소에서 사용 가능"}
          </StatusBadge>
        }
        eyebrow={
          <Link href="/account" className={linkClassName({ variant: "back" })}>
            <Icon name="arrowLeft" size={14} />
            계정
          </Link>
        }
        actions={
          <>
            {data.canRemove ? (
              <RemovePasskeyButton id={id} name={row.name} />
            ) : null}
            <Link
              href={`/account/passkeys/${encodeURIComponent(id)}/edit`}
              className={buttonClassName({ variant: "secondary" })}
            >
              이름 변경
            </Link>
          </>
        }
      />
      {query.added === "1" ? (
        <Banner tone="success" title="패스키를 등록했습니다." />
      ) : query.renamed === "1" ? (
        <Banner tone="success" title="이름을 변경했습니다." />
      ) : null}
      {!data.canRemove ? (
        <Banner
          tone="info"
          title="비밀번호가 없는 외부 로그인 전용 계정에서는 패스키를 삭제할 수 없습니다."
        />
      ) : null}
      <Surface title="패스키 정보">
        <Facts
          items={[
            { term: "이름", value: row.name },
            {
              term: "주소",
              value: row.address,
              note: !row.unavailableHere ? (
                "지금 접속한 주소와 같습니다."
              ) : (
                <>
                  <StatusBadge tone="neutral" size="sm">
                    이 주소에서 사용 불가
                  </StatusBadge>
                  {data.currentOrigin
                    ? `지금 접속한 주소(${new URL(data.currentOrigin).host})와 프로토콜 또는 주소·포트가 달라 여기서는 로그인에 쓸 수 없습니다.`
                    : "이 접속에서는 패스키를 쓸 수 없습니다. 이유는 계정 페이지에서 볼 수 있습니다."}
                </>
              ),
            },
            {
              term: "보관 방식",
              value: row.backupEligible ? "동기화 가능" : "이 기기 전용",
              note: row.backupEligible
                ? "같은 계정으로 동기화되는 다른 기기에서도 쓸 수 있습니다."
                : "등록한 기기나 보안 키 하나에만 보관됩니다.",
            },
            {
              term: "백업 상태",
              value: (
                <StatusBadge
                  tone={
                    row.backupEligible && row.backupState
                      ? "success"
                      : "neutral"
                  }
                >
                  {row.backupEligible
                    ? row.backupState
                      ? "백업됨"
                      : "아직 백업되지 않음"
                    : "백업할 수 없음"}
                </StatusBadge>
              ),
              note: "등록 시 또는 마지막 패스키 로그인에서 확인한 상태입니다. 현재 상태는 비밀번호 관리자에서 확인하세요.",
            },
            { term: "등록일", value: row.createdAt },
            { term: "최근 사용", value: row.lastUsedAt },
          ]}
        />
      </Surface>
    </section>
  );
}
