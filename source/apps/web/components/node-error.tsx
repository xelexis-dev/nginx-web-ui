import { Icon, StatusBadge, type Tone, buttonClassName, Surface, Banner } from "@repo/ui";
import Link from "next/link";
import { describeMismatch } from "@repo/presentation/version";
import { APP_SEMVER } from "@/lib/version";
import type { NodeFailure, NodeStatus } from "@/lib/node-result";
import styles from "./node-error.module.css";

/**
 * Shared by server-rendered node pages and client form error states.
 * Runtime helpers come from the pure presentation package; node result/status
 * imports are types only. No database, credentials, or remote client is imported.
 */

/** Gateway permission denial and a remote node 403 share the same user-facing message. */
export const NODE_FORBIDDEN_MESSAGE = "이 작업을 수행할 권한이 없습니다.";

/**
 * 노드가 2xx 를 냈는데 몸통이 예상과 다를 때의 문구. **읽기와 쓰기가 같은 문구를
 * 써야 한다** — 사용자에게는 같은 사건이고, 화면이 어느 경로로 그 상태에 닿았는지는
 * 알 바가 아니다. `lib/node-page.tsx`(읽기: `toViewModel` 이 던짐)와
 * Rust 노드 작업 응답(쓰기: 성공 응답에서 값을 못 꺼냄) 둘 다 이걸 쓴다.
 */
export const NODE_UNEXPECTED_RESPONSE_MESSAGE =
  "노드가 예상과 다른 응답을 보냈습니다.";

export interface NodeErrorCardProps {
  failure: NodeFailure;
  /** `/nodes/[id]/edit` 링크(unauthorized) 를 만드는 데 쓴다. */
  nodeId: string;
  /**
   * "다시 시도"(unreachable·node_error) 가 갈 곳 — **지금 이 화면의 주소**다.
   * 재시도는 같은 화면을 다시 여는 것이므로 상세든 폼이든 자기 주소를 넘긴다.
   */
  retryHref: string;
  /**
   * "목록으로 돌아가기"(not_found) 가 갈 곳 — **이 화면이 보던 항목들의 목록**이다.
   *
   * **`retryHref` 와 겸용하지 않는다.** 예전에는 하나로 썼는데("사라진 항목의
   * 목록으로도 결국 이 화면의 정상 상태 주소"), 그 논리는 **목록 화면에서만**
   * 참이다. 상세·수정·삭제에서는 그 항목이 이미 없어서 카드가 뜬 것이라, 같은
   * 주소로 돌려보내면 **같은 카드가 다시 뜬다** — 사이드바 말고는 빠져나갈 길이
   * 없다. 쓰기 경로(폼 안의 카드, 상세의 삭제)에서는 더 나쁘다: 화면은 멀쩡히
   * 떠 있는데 유일한 탈출구가 막힌 것으로 보인다.
   *
   * **optional 로 두지 마라.** 빠뜨린 화면이 조용히 잘못된 곳으로 가고, 그건
   * `retryHref` 를 필수로 만든 것과 **같은 계열의 결함**이다
   * (`lib/node-action.ts` 의 `RunNodeWriteOptions` 문서).
   *
   * **목록 화면은 `retryHref` 와 같은 값을 넘긴다** — 우연이 아니라 단언이다.
   */
  listHref: string;
  /**
   * **이 카드가 말하는 대상**(예: `"그 서버를"`, `"인증서 목록을"`). `not_found`
   * 갈래에서만 쓰인다.
   *
   * **조사까지 포함한 목적어구를 받는다** — 「… 노드에서 찾을 수 없습니다」의
   * 목적어 자리에 그대로 들어간다. 여기서 `을(를)` 을 붙이지 않는 이유: 이 값은
   * 사용자 데이터가 아니라 **개발자가 쓴 다섯 개의 상수**(각 화면의
   * `subjects`)라, 어느 조사가 맞는지 쓰는 사람이 그 자리에서 안다.
   * (`history/rollback-button.tsx` 가 `을(를)` 을 쓰는 것은 거기 들어가는 것이
   * **운영자가 지은 노드 이름**이라 컴파일 시점에 알 수 없기 때문이다.)
   *
   * **왜 있나(계획 4b Task 4 결정).** 한 화면이 노드 엔드포인트를 **셋** 부르기
   * 시작하면서(`loadNodePageAll`) 카드 하나가 여러 조회를 대표하게 됐다. 그런데
   * 「그 항목이 노드에 없습니다」는 **어느 조회가 404 였는지 말해주지 않는다** —
   * 서버 상세에서 인증서 조회가 404 여도 사용자는 「이 서버가 사라졌다」로 읽고,
   * 그 밑의 「목록으로 돌아가기」가 그 오해를 확인해 준다. 카드가 무엇이 없는지
   * 말하면 그 링크는 다시 **지금 있던 화면에서 빠져나가는 길**이라는 원래 뜻이
   * 된다.
   *
   * **경로별 `listHref` 를 두는 대신 이걸 골랐다.** 실패한 조회의 리소스 목록으로
   * 보내면(예: 서버를 고치다 인증서 404 → 인증서 목록) 사용자를 하던 일에서
   * 다른 리소스로 쫓아낸다. 사용자가 알아야 하는 것은 **무엇이 없는지**이고,
   * 갈 곳은 원래 하던 화면의 목록이 맞다.
   *
   * optional 인 이유: 경로가 하나인 화면(`loadNodePage`)과 쓰기 액션
   * (`runNodeWrite`)에는 대상이 하나뿐이라 덧붙일 문맥이 없다 — 그때는 기존
   * 문구가 그대로 정확하다. **다중 경로 형태에서는 optional 이 아니다**:
   * `LoadNodePageAllArgs.subjects` 가 경로마다 하나씩 **필수로** 요구한다
   * (경로를 더하면서 문맥을 빠뜨릴 수 없다).
   */
  subject?: string;
  /**
   * `domain_failure`(422) 배너의 **제목**. 기본값은 적용 경로의 문구
   * (`DEFAULT_DOMAIN_FAILURE_TITLE`)다.
   *
   * **왜 갈아끼울 수 있어야 하나:** 422 는 「`nginx -t` 실패 **등** 도메인 실패」
   * 이고(스펙 §9 표), 노드는 인증서 **갱신·갱신 시작·TXT 검증 시작·교체**도
   * 같은 코드로 낸다 — nginx 가 본 적도 없는 실패다. 고정 문구로 두면 ACME
   * 실패가 「nginx -t 실패」로 그려져 **운영자를 엉뚱한 곳으로 보낸다.** 본문은
   * 어차피 노드가 준 원문이라 정확한데, 제목만 거짓말을 한다.
   *
   * **규칙: 제목은 실패한 「동작」을 부르고, 게이트웨이가 관측하지 않은
   * 하위 시스템을 절대 이름 대지 않는다.** 적용·롤백은 기본값을 그대로 쓴다
   * (그 경로는 실제로 `nginx -t` 를 관측한 결과다). 상세는 노드가 준 `message`
   * 가 아래 `<pre>` 로 나른다 — 제목에 사연을 적지 마라.
   *
   * optional 인 이유는 `subject` 와 같다: 대부분의 호출부는 기본값이 맞고,
   * 틀린 자리만 자기 문구를 준다.
   */
  domainFailureTitle?: string;
  nodeErrorTitle?: string;
}

/**
 * 적용·롤백 경로의 422 제목. **이 문구는 `nginx -t` 를 실제로 거친 실패에만
 * 쓴다** — 그 밖의 호출부는 `domainFailureTitle` 로 자기 동작을 이름 댄다.
 */
export const DEFAULT_DOMAIN_FAILURE_TITLE = "설정 적용 실패 — nginx -t 실패";

/**
 * 스펙 §9 표를 그대로 옮긴 실패 카드. 캐시가 없는 노드 화면에서 호출이
 * 실패하면 **이 카드가 화면의 전부**가 되는 경우가 많다 — 문구를 표에서 벗어나
 * 지어내지 않는다.
 *
 * `kind` 마다 생김새가 갈린다:
 * - 구분선 상태(톤 아이콘 + 제목 + 설명 + 선택적 이동): 이 화면 말고 보여줄 게
 *   없는 경우. unreachable·unauthorized·forbidden·version_mismatch·too_old·
 *   not_found·node_error.
 * - 배너(로컬 apply 배너와 같은 모양, 테두리 전체 톤): 폼/목록은 이미 떠 있고
 *   액션 하나만 거부된 경우. conflict·rate_limited·domain_failure.
 *   **conflict 는 일부러 카드가 아니라 배너다** — version_mismatch 와 둘 다
 *   409 지만 뜻이 다르다(Rust `nodes/client.rs` 의 실패 분류). version_mismatch 는
 *   노드를 다시 붙일 때까지 남는 배지가 딸린 지속 상태고, conflict 는 사용자가
 *   고치면 사라지는 일시적 실패라 노드 상태를 건드리지 않는다(Rust 노드 클라이언트가 충돌을 지속적인 연결 상태로 저장하지 않는다).
 *   그래서 이 컴포넌트도 둘을 생김새부터 다르게 그린다 —
 *   나중에 누가 conflict 를 카드로 "정리" 하면 그 구분이 지워진다.
 * - 아무것도 그리지 않음: validation. 폼 필드에 붙는 종류라 카드로 띄우면
 *   안 되는데, 호출부가 실수로 넘기면 조용히 사라지는 대신 콘솔에 알린다.
 */
/** One source for card text and apply-result toasts, including their escape links. */
export function nodeErrorPresentation({
  failure, nodeId, retryHref, listHref, subject, nodeErrorTitle,
  domainFailureTitle = DEFAULT_DOMAIN_FAILURE_TITLE,
}: NodeErrorCardProps): {
  title: string; body?: string; link?: string; linkLabel?: string;
} | null {
  switch (failure.kind) {
    case "unreachable":
      return { title: "노드에 연결할 수 없습니다", body: failure.message, link: retryHref, linkLabel: "다시 시도" };
    case "unauthorized":
      return { title: "이 노드가 연결을 끊었습니다", body: "다시 연결하세요.", link: `/nodes/${nodeId}/edit`, linkLabel: "다시 연결하기" };
    // Only this action is forbidden; its resource list can still be opened.
    case "forbidden":
      return { title: NODE_FORBIDDEN_MESSAGE, link: listHref, linkLabel: "목록으로 돌아가기" };
    // Every page on an incompatible node repeats this error; exit to /nodes.
    case "version_mismatch":
      return { title: "버전이 맞지 않습니다", body: describeMismatch(APP_SEMVER, failure.nodeVersion ?? ""), link: "/nodes", linkLabel: "노드 목록으로" };
    // This node has no feature support; a sibling page would repeat the same card.
    case "too_old":
      return { title: "노드 기능을 사용할 수 없습니다", body: failure.message, link: "/nodes", linkLabel: "노드 목록으로" };
    // Retrying a missing item loops; return to its list instead.
    case "not_found":
      return { title: subject ? `${subject} 노드에서 찾을 수 없습니다` : "그 항목이 노드에 없습니다", body: "이미 지워졌을 수 있습니다.", link: listHref, linkLabel: "목록으로 돌아가기" };
    case "node_error":
      return { title: nodeErrorTitle ?? "노드에서 오류가 발생했습니다",
        body: nodeErrorTitle ? undefined : "노드의 로그를 확인하세요.",
        link: retryHref, linkLabel: "다시 시도" };
    case "conflict":
    case "rate_limited":
      return { title: failure.message };
    // The node envelope carries nginx output in message, not a separate output field.
    case "domain_failure":
      return { title: domainFailureTitle, body: failure.message };
    default:
      return null;
  }
}

export function NodeErrorCard(props: NodeErrorCardProps) {
  const { failure } = props;
  const message = nodeErrorPresentation(props);
  if (!message) {
    // Validation belongs beside form fields; it must never become a page error card.
    if (failure.kind === "validation") {
      console.warn(
        "NodeErrorCard: validation 실패는 폼 필드에 붙여야 합니다. 카드로 렌더하지 않습니다.",
        failure,
      );
    }
    return null;
  }
  if (failure.kind === "domain_failure" || failure.kind === "conflict" || failure.kind === "rate_limited") {
    return (
      <Banner tone={failure.kind === "domain_failure" ? "error" : "warning"}
        title={<>{message.title}</>} role={failure.kind === "domain_failure" ? "alert" : "status"}
        output={failure.kind === "domain_failure" ? message.body : undefined} />
    );
  }
  const tone = failure.kind === "forbidden" || failure.kind === "not_found"
    ? "neutral"
    : failure.kind === "version_mismatch" || failure.kind === "too_old"
      ? "warning" : "error";
  const toneClass = tone === "neutral" ? styles.cardNeutral : tone === "warning" ? styles.cardWarning : styles.cardError;
  return (
    <Surface className={toneClass}><div className={styles.content} role={tone === "error" ? "alert" : "status"}>
      <p className={styles.title}><span className={styles.icon}><Icon name={failure.kind === "version_mismatch" || failure.kind === "too_old" ? "alert" : "circleAlert"} size={16} /></span>{message.title}</p>
      {message.body !== undefined ? <p className={styles.body}>{message.body}</p> : null}
      <div className={styles.actions}>
        <Link href={message.link!} className={buttonClassName({ variant: "secondary" })}>
          {message.linkLabel}
        </Link>
      </div>
    </div></Surface>
  );
}

const STATUS_LABEL: Record<NodeStatus, string> = {
  ok: "응답함",
  unreachable: "응답 없음",
  unauthorized: "연결 끊김",
  version_mismatch: "버전 불일치",
  error: "오류",
};

// `noUncheckedIndexedAccess` 가 켜져 있어 CSS 모듈 클래스 조회는 전부
// `string | undefined` 다(실제로는 항상 있다 — 아래 클래스들은 node-error.module.css
// 에 실재한다). `?? ""` 는 그 타입을 좁히는 것일 뿐 실제로 undefined 가 될 일은 없다.
const STATUS_TONE: Record<NodeStatus, Tone> = {
  ok: "success",
  unreachable: "error",
  unauthorized: "error",
  version_mismatch: "warning",
  error: "error",
};

export interface NodeStatusBadgeProps {
  /** `nodes.lastStatus`. 아직 한 번도 확인되지 않은 새 노드는 null 이다. */
  status: NodeStatus | null;
  /** `nodes.lastVersion`. 있으면 점 옆에 덧붙인다. */
  version?: string | null;
}

/** 노드 목록의 상태 점(스펙 §7 목업의 `● 응답함 v1.15.0`). */
export function NodeStatusBadge({ status, version }: NodeStatusBadgeProps) {
  const label = status ? STATUS_LABEL[status] : "확인 전";
  // 상태를 아직 모르면(null) 톤 클래스를 붙이지 않는다 — .dot 기본값(muted)이 그대로 남는다.
  const tone = status ? STATUS_TONE[status] : "neutral";

  return (
    <StatusBadge tone={tone} size="sm">
      {label}
      {version ? (
        <span className={styles.badgeVersion}> · v{version}</span>
      ) : null}
    </StatusBadge>
  );
}
