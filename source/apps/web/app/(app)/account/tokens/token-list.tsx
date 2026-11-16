import {
  EmptyState,
  StatusBadge,
  type Tone,
  Surface,
  Table,
  Th,
  Td,
  Tr,
  Facts,
  PageHeader,
  Icon,
  buttonClassName,
} from "@repo/ui";
import Link from "next/link";
import layout from "@/components/page-layout.module.css";
import { STATUS_LABELS, type TokenRowView, type TokenStatus } from "./tokens";
import styles from "./tokens.module.css";

/**
 * 토큰 목록 표. **훅이 없다** — 서버 컴포넌트로도 쓰이고 검사가 직접 부를 수도 있다.
 *
 * ⚠ **`Date` 를 받지 않는다.** 시각은 `tokenPresentation` 이 **서버에서** 이미 문자열로
 * 만들어 둔 것이다(`tokens.ts` 의 `formatDateTime` 주석 참조 — 클라이언트에서 포맷하면
 * 목록과 상세가 서로 다른 시각을 보인다, 실측됨).
 *
 * `basePath` 를 받는 이유는 Task 6(관리자 서버 토큰 화면)이 같은 표를
 * `/admin/tokens` 아래에서 쓰기 때문이다. 경로를 박으면 그 화면의 행이 사용자
 * 화면으로 새어 나간다.
 */
export interface TokenListProps {
  rows: TokenRowView[];
  /** 목록이 사는 경로. 행 링크는 `${basePath}/${row.id}` 다. */
  basePath: string;
  /** 비어 있을 때의 문구 — 종류에 따라 다르다("아직 만든 토큰이 없습니다" 등). */
  emptyMessage?: string;
}

const BADGE_TONE: Record<TokenStatus, Tone> = {
  active: "success",
  expired: "warning",
  revoked: "error",
};

export function TokenList({
  rows,
  basePath,
  emptyMessage = "아직 만든 토큰이 없습니다.",
}: TokenListProps) {
  return (
    <Surface flush>
      <Table density="comfortable" narrow="stacked" label="액세스 토큰">
        <thead>
          <Tr>
            <Th>이름</Th>
            <Th>토큰</Th>
            <Th>권한</Th>
            <Th>마지막 사용</Th>
            <Th>만료</Th>
            <Th>상태</Th>
          </Tr>
        </thead>
        <tbody>
          {rows.length === 0 ? (
            <Tr>
              <Td colSpan={6}>
                <EmptyState description={emptyMessage} />
              </Td>
            </Tr>
          ) : null}
          {rows.map((row) => (
            <Tr key={row.id}>
              <Td cell="primary" icon={<Icon name="tokens" size={18} />}>
                {/*
                 * ⚠ **링크 대상은 `row.id` 다.** `masked` 는 접두사 + 뒤 4자라
                 * 22비트뿐이고 약 2,400개에서 겹친다(Task 4 실측) — 식별자로 쓰면
                 * 남의 토큰 상세로 가는 날이 온다.
                 */}
                <Link href={`${basePath}/${row.id}`}>{row.name}</Link>
              </Td>
              {/* 평문은 여기 없다. `masked` 는 목록이 가진 전부이고 그것으로 충분하다. */}
              <Td cell="meta" label="토큰" className={styles.mono}>
                {row.masked}
              </Td>
              <Td cell="meta" label="권한">
                <ul className={styles.scopeTags}>
                  {row.scopes.map((scope) => (
                    <li key={scope}>
                      <StatusBadge
                        tone="neutral"
                        size="sm"
                        variant="outline"
                        dot={false}
                      >
                        {scope}
                      </StatusBadge>
                    </li>
                  ))}
                </ul>
              </Td>
              <Td cell="meta" label="마지막 사용">
                {row.lastUsedAt}
              </Td>
              <Td cell="meta" label="만료">
                {row.expiresAt}
              </Td>
              <Td cell="status" label="상태">
                <StatusBadge tone={BADGE_TONE[row.status]} size="sm">
                  {STATUS_LABELS[row.status]}
                </StatusBadge>
              </Td>
            </Tr>
          ))}
        </tbody>
      </Table>
    </Surface>
  );
}

/** The route and Storybook share the same list composition; API reads stay in the route. */
export function AccountTokensView({ rows }: { rows: TokenRowView[] }) {
  const views = rows;
  return (
    <section className={layout.page}>
      <PageHeader
        title={<>액세스 토큰</>}
        info={<>외부 서비스에서 이용 가능한 엑세스 토큰을 관리합니다</>}
        actions={
          <>
            {" "}
            <div className={styles.listActions}>
              <Link
                href="/account/tokens/new"
                className={buttonClassName({ variant: "primary" })}
              >
                <Icon name="plus" />새 토큰
              </Link>
            </div>{" "}
          </>
        }
      />

      <Facts
        layout="strip"
        items={[
          { term: "토큰", value: views.length },
          {
            term: STATUS_LABELS.active,
            value: views.filter((row) => row.status === "active").length,
          },
          {
            term: STATUS_LABELS.expired,
            value: views.filter((row) => row.status === "expired").length,
          },
          {
            term: STATUS_LABELS.revoked,
            value: views.filter((row) => row.status === "revoked").length,
          },
        ]}
      />
      <TokenList
        rows={views}
        basePath="/account/tokens"
        emptyMessage="아직 만든 토큰이 없습니다."
      />
    </section>
  );
}

/** Same server-token list composition for the route and safe fixtures. */
export function ServerTokensView({ rows }: { rows: TokenRowView[] }) {
  return (
    <section className={layout.page}>
      <PageHeader
        title={<>서버 액세스 토큰</>}
        info={
          <>
            <p>
              이 서버가 발급한 서버 토큰입니다. node:pair 권한이 실린 토큰이
              있어야 다른 게이트웨이가 이 서버를 자기 노드로 연결할 수 있습니다.
              토큰 값은 발급할 때 한 번만 보여줍니다.
            </p>
            <p>
              여기서 내준 토큰은 상대가 연결에 성공하면 「보안 (2FA)」의
              「연결된 게이트웨이」에 한 줄로 남습니다. 토큰을 해지하면 이
              토큰과 연결 기록이 있는 게이트웨이의 접근도 함께 해지됩니다.
              업데이트 전에 연결된 게이트웨이는 이 토큰과의 연결 정보가 없어
              함께 해지되지 않으니, 「보안 (2FA)」의 「연결된 게이트웨이」에서
              해지하세요.
            </p>
          </>
        }
        actions={
          <>
            {" "}
            <div className={styles.listActions}>
              <Link
                href="/admin/tokens/new"
                className={buttonClassName({ variant: "primary" })}
              >
                <Icon name="plus" />새 서버 토큰
              </Link>
            </div>{" "}
          </>
        }
      />

      <TokenList
        rows={rows}
        basePath="/admin/tokens"
        emptyMessage="아직 발급한 서버 토큰이 없습니다. 다른 게이트웨이가 이 서버에 연결하게 하려면 node:pair 권한을 가진 토큰을 발급해 그쪽에 전달하세요."
      />
    </section>
  );
}
