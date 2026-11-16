import type { ReactNode } from "react";
import Link from "next/link";
import {
  EmptyState,
  Icon,
  StatusBadge,
  Surface,
  Table,
  Td,
  Th,
  Tr,
  buttonClassName,
} from "@repo/ui";
import type { PasskeyListView, PasskeyRowView } from "@/lib/passkey-api";
import { PasskeyAvailabilityNotice } from "./passkey-availability-notice";
import styles from "./passkeys.module.css";

export function PasskeyAddress({ row }: { row: PasskeyRowView }) {
  return (
    <span className={styles.address}>
      {row.address}
      {row.unavailableHere ? (
        <StatusBadge tone="neutral" size="sm">
          이 주소에서 사용 불가
        </StatusBadge>
      ) : null}
    </span>
  );
}
export function PasskeyTable({
  rows,
  action,
}: {
  rows: PasskeyRowView[];
  action?: (row: PasskeyRowView) => ReactNode;
}) {
  return (
    <Table label="패스키" density="comfortable" layout="fixed" narrow="stacked">
      <colgroup>
        <col className={styles.nameColumn} />
        <col />
        <col className={styles.storageColumn} />
        <col className={styles.createdColumn} />
        <col className={styles.usedColumn} />
        {action ? <col className={styles.actionColumn} /> : null}
      </colgroup>
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
        {rows.length ? (
          rows.map((row) => (
            <Tr key={row.id}>
              <Td cell="primary" wrap>
                {action ? (
                  row.name
                ) : (
                  <Link
                    href={`/account/passkeys/${encodeURIComponent(row.id)}`}
                  >
                    {row.name}
                  </Link>
                )}
              </Td>
              <Td wrap label="주소">
                <PasskeyAddress row={row} />
              </Td>
              <Td label="보관 방식">
                {row.backupEligible ? "동기화 가능" : "이 기기 전용"}
              </Td>
              <Td label="등록일">{row.createdOn}</Td>
              <Td label="최근 사용">{row.lastUsedAt}</Td>
              {action ? (
                <Td cell="action" label="작업">
                  {action(row)}
                </Td>
              ) : null}
            </Tr>
          ))
        ) : (
          <Tr>
            <Td colSpan={action ? 6 : 5}>
              <EmptyState description="등록한 패스키가 없습니다." />
            </Td>
          </Tr>
        )}
      </tbody>
    </Table>
  );
}
export function AccountPasskeys({
  data,
  role = "viewer",
}: {
  data: PasskeyListView;
  role?: string;
}) {
  const count = data.passkeys.length;
  const describe = data.hasPassword || count > 0;
  return (
    <Surface
      id="account-passkeys"
      title="패스키"
      tag={
        describe ? (
          <StatusBadge tone="neutral" dot={false} size="sm">
            {count}/10
          </StatusBadge>
        ) : undefined
      }
      description={
        describe ? (
          <>
            패스키는 등록한 주소에서만 쓸 수 있습니다.
            {data.availability.available ? (
              <>
                {" "}
                지금 접속한 주소는 <strong>{data.currentAddress}</strong>입니다.
              </>
            ) : null}
            {count === 10
              ? " 10개를 모두 등록했습니다. 새로 등록하려면 사용하지 않는 패스키를 삭제하세요."
              : null}
          </>
        ) : undefined
      }
      actions={
        data.hasPassword && data.availability.available && count < 10 ? (
          <Link
            href="/account/passkeys/new"
            className={buttonClassName({ variant: "secondary" })}
          >
            <Icon name="plus" />
            패스키 등록
          </Link>
        ) : undefined
      }
    >
      <div className={styles.sectionBody}>
        {!data.availability.available ? (
          <PasskeyAvailabilityNotice
            reason={data.availability.reason}
            role={role}
          />
        ) : null}
        {count ? (
          <PasskeyTable rows={data.passkeys} />
        ) : !data.hasPassword ? (
          <EmptyState
            title="등록한 패스키가 없습니다"
            description="외부 로그인만 쓰는 계정은 패스키를 등록할 수 없습니다."
          />
        ) : !data.availability.available ? (
          <p className={styles.bodyText}>등록한 패스키가 없습니다.</p>
        ) : (
          <EmptyState
            title="등록한 패스키가 없습니다"
            description="패스키를 등록하면 비밀번호를 입력하지 않고 기기의 지문·얼굴 인식이나 화면 잠금으로 로그인할 수 있습니다."
          />
        )}
      </div>
    </Surface>
  );
}
