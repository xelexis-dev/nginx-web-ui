import type { ReactNode } from "react";
import { Icon } from "./icon";
import type { RenderLink } from "./app-shell";
import styles from "./context-bar.module.css";
export interface ContextBarProps {
  name: string;
  offline?: boolean;
  status: ReactNode;
  facts?: ReactNode;
  renderLink?: RenderLink;
}
export function ContextBar({
  name,
  offline = false,
  status,
  facts,
  renderLink,
}: ContextBarProps) {
  const link: RenderLink = renderLink ?? ((props) => <a {...props} />);
  return (
    <section
      className={styles.bar}
      data-offline={offline}
      aria-label="원격 노드 맥락"
    >
      <span className={styles.icon}>
        <Icon name="nodes" size={16} />
      </span>
      <div className={styles.identity}>
        <p className={styles.kicker}>원격 노드</p>
        <strong className={styles.name} title={name}>
          {name}
        </strong>
      </div>
      <div className={styles.status}>{status}</div>
      {facts ? <div className={styles.facts}>{facts}</div> : null}
      <div className={styles.actions}>
        {link({
          href: "/nodes",
          className: styles.link!,
          children: "노드에서 나가기",
        })}
      </div>
    </section>
  );
}
