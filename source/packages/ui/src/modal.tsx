"use client";

import { useEffect, useId, useRef, type ReactNode } from "react";
import { IconButton } from "./button";
import { Icon } from "./icon";
import styles from "./modal.module.css";

/**
 * 공통 모달. 네이티브 `<dialog>` + `showModal()`을 쓰므로 포커스 트랩, Esc 닫기,
 * 배경 inert를 브라우저가 제공한다(직접 구현하지 않는다). 닫힘 경로를 하나로
 * 모으려고 닫기 버튼/배경 클릭도 `dialog.close()`를 호출하고, `close` 이벤트만
 * `onClose`로 올린다.
 */
export interface ModalProps {
  open: boolean;
  onClose: () => void;
  title: string;
  /** Optional semantic warning for risky or destructive actions; informational dialogs omit it. */
  icon?: "warning" | "danger";
  description?: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
}

export function Modal({
  open,
  onClose,
  title,
  icon,
  description,
  children,
  footer,
}: ModalProps) {
  const ref = useRef<HTMLDialogElement | null>(null);
  const titleId = useId();

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    if (open && !el.open) el.showModal();
    if (!open && el.open) el.close();
  }, [open]);

  return (
    <dialog
      ref={ref}
      className={styles.dialog}
      aria-labelledby={titleId}
      aria-describedby={description ? titleId + "-description" : undefined}
      onClose={(event) => {
        // React propagates close events from nested dialogs. Only this dialog's
        // close changes its own open state (e.g. Apply → pending changes).
        if (event.target === event.currentTarget) onClose();
      }}
      // 패널 밖(스크림) 클릭은 target이 dialog 자신이 된다 — 내용 클릭과 구분된다.
      onClick={(e) => {
        if (e.target !== ref.current) return;
        const bounds = e.currentTarget.getBoundingClientRect();
        if (
          e.clientX < bounds.left ||
          e.clientX > bounds.right ||
          e.clientY < bounds.top ||
          e.clientY > bounds.bottom
        )
          ref.current?.close();
      }}
    >
      <div className={styles.header}>
        {icon ? (
          <span className={styles.icon} data-tone={icon}>
            <Icon name="alert" size={20} />
          </span>
        ) : null}
        <div className={styles.heading}>
          <h2 id={titleId} className={styles.title}>
            {title}
          </h2>
          {description ? (
            <div id={titleId + "-description"} className={styles.description}>
              {description}
            </div>
          ) : null}
        </div>
        <IconButton
          label="닫기"
          variant="ghost"
          className={styles.close}
          onClick={() => ref.current?.close()}
        >
          <Icon name="close" />
        </IconButton>
      </div>
      <div className={styles.body}>{children}</div>
      {footer ? <div className={styles.footer}>{footer}</div> : null}
    </dialog>
  );
}
