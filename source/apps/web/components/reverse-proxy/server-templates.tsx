import type { ReactNode } from "react";
import layout from "@/components/page-layout.module.css";
import Link from "next/link";
import {
  Surface,
  StatusBadge,
  CodePane,
  PageHeader,
  buttonClassName,
  linkClassName,
} from "@repo/ui";
import { configPreview, type ConfigPreview } from "@/lib/config-preview";
import styles from "@/app/(app)/reverse-proxy/reverse-proxy.module.css";

export function ServerTemplates({
  items,
  canWrite,
  navigation,
}: {
  items: {
    id: string;
    name: string;
    description: string;
    tags: string[];
    preview?: ConfigPreview;
  }[];
  canWrite: boolean;
  navigation?: ReactNode;
}) {
  return (
    <section className={layout.page}>
      <PageHeader
        eyebrow={
          <Link
            href="/reverse-proxy"
            className={linkClassName({ variant: "back" })}
          >
            리버스 프록시
          </Link>
        }
        title="서버 템플릿"
        info="자주 쓰는 구성을 템플릿으로 골라 리버스 프록시 서버를 간편하게 생성합니다. 선택 후 폼에서 자유롭게 수정할 수 있습니다."
        actions={
          canWrite ? (
            <Link
              href="/reverse-proxy/new"
              className={buttonClassName({ variant: "secondary" })}
            >
              빈 폼으로 생성
            </Link>
          ) : undefined
        }
      />

      {navigation}
      <div className={layout.cards}>
        {items.map((t) => {
          const { config, error, unresolvedPaths } = configPreview(t.preview);
          return (
            <Surface key={t.id} title={<>{t.name}</>} as="div">
              <div className={styles.templateTags}>
                {t.tags.map((tag) => (
                  <StatusBadge
                    key={tag}
                    tone="neutral"
                    variant="outline"
                    size="sm"
                    dot={false}
                  >
                    {tag}
                  </StatusBadge>
                ))}
              </div>
              <p className={styles.templateDesc}>{t.description}</p>
              <details className={styles.templatePreview}>
                <summary className={styles.templatePreviewSummary}>
                  생성될 nginx 설정 미리보기
                </summary>
                {unresolvedPaths?.length ? (
                  <div role="status" className={styles.previewHint}>
                    이름 확인 없이 만든 미리보기입니다 — 실제 적용에서는 이
                    경로의 결과가 달라질 수 있습니다
                    <ul className={styles.opsTargets}>
                      {unresolvedPaths.map((path, index) => (
                        <li className={styles.opsTarget} key={index}>
                          <code>{path}</code>
                        </li>
                      ))}
                    </ul>
                  </div>
                ) : null}
                <CodePane
                  title={t.name}
                  code={config ?? undefined}
                  error={error ?? undefined}
                />
              </details>
              {canWrite ? (
                <Link
                  href={`/reverse-proxy/new?template=${t.id}`}
                  className={buttonClassName({ variant: "secondary" })}
                >
                  이 템플릿으로 생성
                </Link>
              ) : null}
            </Surface>
          );
        })}
      </div>
    </section>
  );
}
