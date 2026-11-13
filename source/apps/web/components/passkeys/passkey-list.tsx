import type { ReactNode } from "react";
import Link from "next/link";
import {
  Banner,
  Button,
  EmptyState,
  Icon,
  StatusBadge,
  Card,
  CardFoot,
  Table,
  Td,
  Th,
  Tr,
  buttonClassName,
  linkClassName,
} from "@repo/ui";
import type { PasskeyListView, PasskeyRowView } from "@/lib/passkey-api";
import cards from "@/components/account-cards.module.css";
import styles from "./passkeys.module.css";

function PasskeyUnavailableText() {
  return (
    <>
      패스키를 사용하려면 HTTPS 도메인 주소 또는 설치한 컴퓨터의
      http://localhost로 접속하세요.{" "}
      <a
        className={linkClassName()}
        href="https://github.com/xelexis-dev/nginx-web-ui/blob/main/docs/README.ko.md#3-최초-관리자-생성"
        target="_blank"
        rel="noreferrer"
      >
        설치 안내 보기
      </a>
    </>
  );
}
export function PasskeyUnavailable() {
  return (
    <p className={styles.hint}>
      <PasskeyUnavailableText />
    </p>
  );
}
export function PasskeyAddress({
  row,
  available,
}: {
  row: PasskeyRowView;
  available: boolean;
}) {
  return (
    <>
      {row.address}
      {available && row.unavailableHere ? (
        <StatusBadge
          className={styles.badge}
          tone="neutral"
          dot={false}
          size="sm"
        >
          이 주소에서 사용 불가
        </StatusBadge>
      ) : null}
    </>
  );
}
export function PasskeyTable({
  rows,
  available,
  action,
}: {
  rows: PasskeyRowView[];
  available: boolean;
  action?: (row: PasskeyRowView) => ReactNode;
}) {
  return (
    <Table label="패스키">
      <thead>
        <Tr>
          <Th>이름</Th>
          <Th>주소</Th>
          <Th>보관 방식</Th>
          <Th>등록일</Th>
          <Th>최근 사용</Th>
          {action ? <Th>작업</Th> : null}
        </Tr>
      </thead>
      <tbody>
        {rows.map((row) => (
          <Tr key={row.id}>
            <Td cell="primary" icon={<Icon name="tokens" size={18} />}>
              {action ? (
                <strong>{row.name}</strong>
              ) : (
                <Link href={`/account/passkeys/${encodeURIComponent(row.id)}`}>
                  {row.name}
                </Link>
              )}
            </Td>
            <Td wrap label="주소">
              <PasskeyAddress row={row} available={available} />
            </Td>
            <Td label="보관 방식">
              <StatusBadge tone="neutral" dot={false} size="sm">
                {row.backupEligible ? "동기화 가능" : "이 기기 전용"}
              </StatusBadge>
            </Td>
            <Td label="등록일">{row.createdAt}</Td>
            <Td label="최근 사용">{row.lastUsedAt}</Td>
            {action ? <Td cell="action">{action(row)}</Td> : null}
          </Tr>
        ))}
      </tbody>
    </Table>
  );
}
export function AccountPasskeys({
  data,
  removed = false,
}: {
  data: PasskeyListView;
  removed?: boolean;
}) {
  const count = data.passkeys.length;
  const disabled =
    !data.availability.available || !data.hasPassword || count >= 10;
  const register = disabled ? (
    <Button variant="secondary" disabled>
      <Icon name="plus" />
      패스키 등록
    </Button>
  ) : (
    <Link
      href="/account/passkeys/new"
      className={buttonClassName({ variant: "secondary" })}
    >
      <Icon name="plus" />
      패스키 등록
    </Link>
  );
  return (
    <Card
      id="account-passkeys"
      footer={
        <CardFoot>
          <span>
            {data.currentAddress ? `현재 주소: ${data.currentAddress} · ` : ""}
            이름을 누르면 상세 정보를 볼 수 있습니다.
          </span>
          <span>최대 10개</span>
        </CardFoot>
      }
      title={
        <span className={styles.title}>
          패스키
          <StatusBadge tone="neutral" dot={false} size="sm">
            {count} / 10
          </StatusBadge>
        </span>
      }
      actions={count ? register : undefined}
    >
      <div className={cards.content}>
        {removed ? (
          <div>
            <Banner tone="success" title="패스키를 삭제했습니다." />
          </div>
        ) : null}
        {!data.availability.available ? (
          <div>
            <Banner
              tone="info"
              icon="info"
              title={<PasskeyUnavailableText />}
            />
          </div>
        ) : null}
        {!data.hasPassword ? (
          <div>
            <p className={styles.hint}>
              외부 로그인 전용 계정은 패스키를 등록할 수 없습니다.
            </p>
          </div>
        ) : null}
        {count >= 10 ? (
          <div>
            <Banner
              tone="warning"
              title="패스키는 최대 10개까지 등록할 수 있습니다."
            >
              새로 등록하려면 사용하지 않는 패스키를 삭제하세요.
            </Banner>
          </div>
        ) : null}
        {count ? (
          <PasskeyTable
            rows={data.passkeys}
            available={data.availability.available}
          />
        ) : (
          <EmptyState
            title="등록된 패스키가 없습니다"
            description="기기의 지문·얼굴 또는 PIN으로 로그인할 수 있습니다."
            action={register}
          />
        )}
      </div>
    </Card>
  );
}
