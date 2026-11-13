"use client";
import { Field, Card, TextInput, StatusBadge, Button } from "@repo/ui";
import formStyles from "@/components/reverse-proxy/server-form.module.css";
export function IdentityStrip({
  nodeName,
  serverName,
  onServerNameChange,
  listen,
  onListenChange,
  https,
  uncovered,
  fieldErrors,
  tlsPort,
  certificate,
  redirectHttp,
  onTls,
}: {
  nodeName?: string;
  serverName: string;
  onServerNameChange: (value: string) => void;
  listen: string;
  onListenChange: (value: string) => void;
  tlsPort: string;
  certificate?: string;
  redirectHttp: boolean;
  onTls: () => void;
  https: boolean;
  uncovered: number;
  fieldErrors?: Record<string, string>;
}) {
  return (
    <Card
      id="server-form-identity"
      title="서버"
      errorCount={
        [fieldErrors?.serverName, fieldErrors?.listen].filter(Boolean).length
      }
    >
      <div className={formStyles.pfCategory}>
        {nodeName ? (
          <p className={formStyles.pfNodeContext}>
            저장 위치: 노드 <strong>{nodeName}</strong>
          </p>
        ) : null}
        <div className={formStyles.pfIdentityFields}>
          <Field
            label="server_name"
            required
            htmlFor="server-name"
            hint="여러 이름은 공백으로 구분합니다."
            error={fieldErrors?.serverName}
          >
            <TextInput
              name="serverName"
              value={serverName}
              onChange={(e) => onServerNameChange(e.target.value)}
              placeholder="app.example.com"
            />
          </Field>
          <Field
            label="listen (포트)"
            required
            htmlFor="server-listen"
            error={fieldErrors?.listen}
          >
            <TextInput
              name="listen"
              type="number"
              min={1}
              max={65535}
              value={listen}
              onChange={(e) => onListenChange(e.target.value)}
            />
          </Field>
        </div>
        <div className={formStyles.pfIdentityStatus}>
          <span>
            {https
              ? `HTTPS :${tlsPort} · ${certificate ?? "인증서 연결됨"} · HTTP :${listen}${redirectHttp ? "은 리다이렉트" : " 사용 안 함"}`
              : `HTTP :${listen}로 수신`}
          </span>

          {uncovered ? (
            <StatusBadge tone="warning">
              인증서 범위 확인 {uncovered}개
            </StatusBadge>
          ) : null}
          <Button type="button" variant="ghost" size="sm" onClick={onTls}>
            TLS에서 변경
          </Button>
        </div>
      </div>
    </Card>
  );
}
