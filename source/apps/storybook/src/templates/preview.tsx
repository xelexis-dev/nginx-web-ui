import { Banner } from "@repo/ui";
import { Children, useState, type ReactNode } from "react";
import styles from "./templates.module.css";

/** Template actions explain the preview instead of pretending to save product data. */
export function TemplatePreview({ children, className }: { children: ReactNode; className?: string }) {
  const [notice, setNotice] = useState("");
  const [header, ...content] = Children.toArray(children);
  return <div className={className}
    onSubmit={(event) => { event.preventDefault(); setNotice("저장 미리보기입니다. 실제 설정은 변경하지 않았습니다."); }}
    onClickCapture={(event) => {
      const action = (event.target as Element).closest('a[href^="#"], [data-preview-action]');
      if (action) { event.preventDefault(); setNotice(`「${action.textContent?.trim()}」 미리보기입니다. 실제 이동이나 변경은 하지 않았습니다.`); }
    }}>
    {header}
    {/* Keep the live region mounted so later text changes can be announced. */}
    <div className={notice ? undefined : styles.srOnly}>
      <Banner tone="info" title={notice} role="status" onDismiss={notice ? () => setNotice("") : undefined} />
    </div>
    {content}
  </div>;
}
