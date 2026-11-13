import { readPageApi } from "@/lib/api-page";
import type { SecretHealth } from "@repo/contracts";
import type { SecretRow } from "./secrets-health";
import { SecretsHealthView } from "./secrets-health";
export const runtime = "nodejs";
export const dynamic = "force-dynamic";
export default async function SecretsHealthPage() {
  const details = {
    oauth: {
      group: "OAuth 로그인",
      impact: "이 공급자의 로그인 버튼이 로그인 화면에서 사라집니다.",
      recovery:
        "공급자 수정 화면에서 클라이언트 비밀키를 다시 입력하면 됩니다.",
    },
    certificate: {
      group: "인증서",
      impact:
        "인증서 파일을 다시 쓸 수 없어 갱신이 멈춥니다(이미 쓰인 파일은 만료까지 동작).",
      recovery: "인증서를 다시 발급하거나, 보관하던 개인키로 다시 가져오세요.",
    },
    acme: {
      group: "ACME 계정",
      impact: "새 인증서 발급과 자동 갱신이 모두 멈춥니다.",
      recovery: "계정 키를 복구할 수 없으면 ACME 계정을 다시 만들어야 합니다.",
    },
    smtp: {
      group: "메일(SMTP)",
      impact: "메일 발송 인증에 실패하여 인증서 만료 안내가 나가지 않습니다.",
      recovery: "SMTP 설정에서 비밀번호를 다시 입력하세요.",
    },
    webhook: {
      group: "Webhook",
      impact: "이 수신처로 알림을 보낼 수 없습니다.",
      recovery: "수신처의 URL과 인증 헤더를 다시 입력하세요.",
    },
    totp: {
      group: "2단계 인증",
      impact:
        "인증 앱 코드로 로그인할 수 없습니다. 백업 코드는 별도로 사용할 수 있습니다.",
      recovery: "관리자가 2단계 인증을 해제한 뒤 다시 등록하세요.",
    },
  };
  const status = await readPageApi<SecretHealth[]>("/api/ui/v1/admin/secrets");
  const rows: SecretRow[] = status.map((row) => ({
    ...details[row.kind],
    label: row.label,
    state: row.state,
    href: row.href ?? undefined,
  }));
  const broken = rows.filter((r) => r.state === "sealed-broken");
  const plaintext = rows.filter((r) => r.state === "plaintext");

  return (
    <SecretsHealthView rows={rows} broken={broken} plaintext={plaintext} />
  );
}
