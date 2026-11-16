"use client";

import type { ReactNode } from "react";
import { Button } from "./button";
import { Modal } from "./modal";
import styles from "./modal.module.css";

export interface ConfirmDialogProps {
  open: boolean;
  onClose: () => void;
  title: string;
  description: ReactNode;
  target?: ReactNode;
  confirmLabel: string;
  onConfirm: () => void;
  /** The caller owns the command, pending state and result. */
  pending?: boolean;
}

export function ConfirmDialog({
  open,
  onClose,
  title,
  description,
  target,
  confirmLabel,
  onConfirm,
  pending = false,
}: ConfirmDialogProps) {
  return (
    <Modal
      open={open}
      dismissible={!pending}
      onClose={onClose}
      title={title}
      icon="danger"
      description={description}
      footer={
        <>
          <Button
            type="button"
            variant="secondary"
            disabled={pending}
            onClick={(event) => event.currentTarget.closest("dialog")?.close()}
          >
            취소
          </Button>
          <Button
            type="button"
            variant="danger"
            disabled={pending}
            aria-busy={pending || undefined}
            onClick={onConfirm}
          >
            {pending ? "처리 중..." : confirmLabel}
          </Button>
        </>
      }
    >
      {target ? (
        <div className={styles.target}>
          <span>대상</span>
          <strong>{target}</strong>
        </div>
      ) : null}
    </Modal>
  );
}
