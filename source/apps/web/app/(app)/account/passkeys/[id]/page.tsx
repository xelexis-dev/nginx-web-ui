import Link from "next/link";
import {
  Banner,
  Facts,
  Icon,
  PageHeader,
  Surface,
  buttonClassName,
  linkClassName,
} from "@repo/ui";
import { readPageApi } from "@/lib/api-page";
import { passkeyRowView, type PasskeyDetail } from "@/lib/passkey-api";
import { PasskeyAddress } from "@/components/passkeys/passkey-list";
import { RemovePasskeyForm } from "@/components/passkeys/passkey-forms";
import layout from "@/components/page-layout.module.css";
export const runtime = "nodejs";
export default async function PasskeyDetailPage({
  params,
  searchParams,
}: {
  params: Promise<{ id: string }>;
  searchParams: Promise<{ added?: string; renamed?: string }>;
}) {
  const { id } = await params;
  const data = await readPageApi<PasskeyDetail>(
    `/api/ui/v1/account/passkeys/${encodeURIComponent(id)}`,
  );
  const row = passkeyRowView(data.passkey);
  const query = await searchParams;
  return (
    <section className={layout.page}>
      <PageHeader
        title={row.name}
        description={row.address}
        eyebrow={
          <Link href="/account" className={linkClassName({ variant: "back" })}>
            <Icon name="arrowLeft" />
            계정으로
          </Link>
        }
        actions={
          <Link
            href={`/account/passkeys/${encodeURIComponent(id)}/edit`}
            className={buttonClassName({ variant: "secondary" })}
          >
            이름 변경
          </Link>
        }
      />
      {query.added === "1" ? (
        <Banner tone="success" title="패스키를 등록했습니다.">
          다음 로그인부터 기기에서 본인을 확인하면 됩니다.
        </Banner>
      ) : query.renamed === "1" ? (
        <Banner tone="success" title="이름을 변경했습니다." />
      ) : null}
      <Surface title="패스키 정보">
        <Facts
          columns={2}
          items={[
            { term: "이름", value: row.name },
            {
              term: "주소",
              value: (
                <PasskeyAddress
                  row={row}
                  available={data.availability.available}
                />
              ),
            },
            {
              term: "보관 방식",
              value: row.backupEligible ? "동기화 가능" : "이 기기 전용",
            },
            {
              term: "백업 상태",
              value: row.backupEligible
                ? row.backupState
                  ? "백업됨"
                  : "아직 백업되지 않음"
                : "백업할 수 없음",
            },
            { term: "등록일", value: row.createdAt },
            { term: "최근 사용", value: row.lastUsedAt },
          ]}
        />
      </Surface>
      <Surface title="패스키 삭제">
        <RemovePasskeyForm
          action={{ id: "account.passkeys.remove", args: [] }}
          id={id}
          canRemove={data.canRemove}
        />
      </Surface>
    </section>
  );
}
