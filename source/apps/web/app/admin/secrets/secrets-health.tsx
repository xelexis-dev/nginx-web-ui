import layout from "@/components/page-layout.module.css";
import {
  StatusBadge,
  Banner,
  EmptyState,
  Surface,
  Table,
  Th,
  Td,
  Tr,
  PageHeader,
} from "@repo/ui";
import Link from "next/link";
import type { SecretHealth } from "@repo/contracts";
type SealedState = SecretHealth["state"];
import styles from "../oauth/oauth.module.css";

/**
 * 저장된 비밀값의 상태를 한 화면에 모은다.
 *
 * **왜 한 자리에 모으나**: 이 값들은 전부 같은 열쇠(`/var/lib/nginx-web-ui/secrets/data_key`)로
 * 열린다. 그 열쇠를 잃으면 알림 메일이 안 나가고, DNS 인증서 갱신이 멈추고,
 * OAuth 로그인 버튼이 사라진다 — 그리고 **셋 다 조용하다.** 기능별 화면에서는
 * 각자 하나씩만 보이므로, 「열쇠가 문제」라는 것을 알아채려면 한 자리에
 * 모아야 한다.
 *
 * 백필(`ENCRYPT_BACKFILL`)을 켤지 판단할 때도 이 화면이 근거가 된다 — 무엇이
 * 아직 평문인지 여기서 한눈에 보인다.
 */

export interface SecretRow {
  group: string;
  label: string;
  state: SealedState;
  /** 이 값이 못 열릴 때 실제로 무엇이 멈추는지. */
  impact: string;
  /** 어떻게 되살리는지. */
  recovery: string;
  href?: string;
}

const STATE_LABEL: Record<SealedState, string> = {
  "sealed-ok": "잠김 · 정상",
  "sealed-broken": "열 수 없음",
  plaintext: "평문",
  absent: "미설정",
};

export function SecretsHealthView({
  rows,
  broken,
  plaintext,
}: {
  rows: SecretRow[];
  broken: SecretRow[];
  plaintext: SecretRow[];
}) {
  return (
    <section className={layout.page}>
      <PageHeader
        title={<>저장된 비밀값</>}
        description={
          <>
            열쇠를 잃으면 메일·인증서 갱신·OAuth 로그인이{" "}
            <strong>동시에, 그리고 조용히</strong> 멈추기 때문에 한 화면에 모아
            둡니다.
          </>
        }
        info={
          <>
            이 값들은 모두 <strong>같은 열쇠</strong>(app-data 볼륨의{" "}
            <code className={styles.mono}>secrets/data_key</code>)로 열립니다.
          </>
        }
      />

      {broken.length > 0 ? (
        <Banner
          tone={"error"}
          title={
            <>
              <strong>{broken.length}개 값을 열 수 없습니다.</strong> app-data
              볼륨의 열쇠 파일이 바뀌었거나 사라졌을 때 생깁니다. 아래 각 항목의
              「되살리는 법」대로 값을 다시 입력하면 복구됩니다.
            </>
          }
          role="alert"
        ></Banner>
      ) : null}

      {plaintext.length > 0 ? (
        <p className={styles.pageNote}>
          <strong>{plaintext.length}개 값이 아직 평문입니다.</strong> 사고가
          아니라 암호화 전환(백필)을 아직 켜지 않은 정상 상태입니다. 다만
          그동안은 데이터베이스 사본만으로 이 값들이 읽힙니다.
        </p>
      ) : null}

      <Surface as="section" flush>
        <Table label="저장된 비밀값">
          <thead>
            <Tr>
              <Th>구분</Th>
              <Th>대상</Th>
              <Th>상태</Th>
              <Th>못 열면 무엇이 멈추나</Th>
            </Tr>
          </thead>
          <tbody>
            {rows.length === 0 ? (
              <Tr>
                <Td colSpan={4}>
                  <EmptyState description={<> 저장된 비밀값이 없습니다. </>} />
                </Td>
              </Tr>
            ) : null}
            {rows.map((r, i) => (
              <Tr key={`${r.group}-${r.label}-${i}`}>
                <Td cell="meta" label="구분">
                  {r.group}
                </Td>
                <Td cell="primary">
                  {r.href ? <Link href={r.href}>{r.label}</Link> : r.label}
                </Td>
                <Td cell="status">
                  <StatusBadge
                    tone={
                      r.state === "sealed-broken"
                        ? "error"
                        : r.state === "sealed-ok"
                          ? "success"
                          : "neutral"
                    }
                    size="sm"
                  >
                    {STATE_LABEL[r.state]}
                  </StatusBadge>
                </Td>
                <Td
                  cell="meta"
                  label="못 열면 무엇이 멈추나"
                  className={styles.rowNote}
                >
                  {r.state === "sealed-broken" ? (
                    <>
                      {r.impact}
                      <br />
                      <strong>되살리는 법:</strong> {r.recovery}
                    </>
                  ) : r.state === "plaintext" ? (
                    "동작에는 문제가 없습니다. 백필을 켜면 잠깁니다."
                  ) : r.state === "absent" ? (
                    "설정되지 않았습니다."
                  ) : (
                    "정상입니다."
                  )}
                </Td>
              </Tr>
            ))}
          </tbody>
        </Table>
      </Surface>

      <Surface title={<>열쇠를 지키는 법</>} as="section">
        <p className={styles.pageNote}>
          열쇠는 <code className={styles.mono}>/var/lib/nginx-web-ui</code> 볼륨
          안에 있습니다. 이 볼륨을 백업 대상에 넣어 두세요 — 데이터베이스만
          백업하면, 복원해도 위 값들을 하나도 열 수 없습니다. 열쇠는 처음 한
          번만 만들어지고 그 뒤로 바뀌지 않으므로,{" "}
          <strong>한 번 뜬 백업이 계속 유효합니다.</strong>
        </p>
      </Surface>
    </section>
  );
}
