"use client";
import { useEffect, useRef, useState, type ReactNode } from "react";
import Link from "next/link";
import { Button, buttonClassName, StatusBadge } from "@repo/ui";
import {
  targetLabel,
  type OperationsTarget,
} from "@repo/presentation/operations-target";
import formStyles from "@/components/reverse-proxy/server-form.module.css";
export interface SummaryItem {
  label: string;
  value: ReactNode;
  onJump: () => void;
  error?: boolean;
}
export function SummaryRail({
  items,
  errors,
  onSaveErrorJump,
  converted,
  pending,
  cancelHref,
  submitLabel,
  target,
}: {
  items: SummaryItem[];
  errors: SummaryItem[];
  onSaveErrorJump?: () => void;
  converted: boolean;
  pending: boolean;
  cancelHref: string;
  submitLabel: string;
  target?: OperationsTarget;
}) {
  const rail = useRef<HTMLElement>(null),
    sheet = useRef<HTMLDialogElement>(null);
  const [open, setOpen] = useState(false);
  const bar = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const element = bar.current;
    if (!element) return;
    const measure = () =>
      element
        .closest("form")
        ?.style.setProperty(
          "--pf-mobile-bar-height",
          `${element.getBoundingClientRect().height}px`,
        );
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    measure();
    return () => observer.disconnect();
  }, []);
  useEffect(() => {
    if (open) sheet.current?.showModal();
    else sheet.current?.close();
  }, [open]);
  useEffect(() => {
    const element = rail.current;
    if (!element) return;
    let frame = 0;
    const measure = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() =>
        element.style.setProperty(
          "--pf-rail-top",
          `${Math.max(16, element.getBoundingClientRect().top)}px`,
        ),
      );
    };
    measure();
    window.addEventListener("scroll", measure, { passive: true });
    window.addEventListener("resize", measure);
    const observer = new ResizeObserver(measure);
    observer.observe(element.parentElement!);
    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
      window.removeEventListener("scroll", measure);
      window.removeEventListener("resize", measure);
    };
  }, []);
  useEffect(() => {
    const narrow = window.matchMedia("(max-width: 768px)");
    const changed = () => {
      if (!narrow.matches) {
        sheet.current?.close();
        setOpen(false);
      }
    };
    narrow.addEventListener("change", changed);
    return () => narrow.removeEventListener("change", changed);
  }, []);
  const close = () => {
    sheet.current?.close();
    setOpen(false);
  };
  const jump = (item: SummaryItem) => {
    close();
    item.onJump();
  };
  const saveError = onSaveErrorJump ? (
    <div className={formStyles.pfSaveError} role="status">
      <span>저장하지 못했습니다.</span>
      <Button
        type="button"
        variant="ghost"
        size="sm"
        onClick={() => {
          close();
          onSaveErrorJump();
        }}
      >
        오류 확인
      </Button>
    </div>
  ) : null;
  const body = (
    <>
      <p className={formStyles.directiveGroupNote}>
        저장할 설정입니다. 저장한 뒤 「설정 적용」 전까지 nginx에는 반영되지
        않습니다.
      </p>
      <dl className={formStyles.pfSummaryList}>
        {items.map((item, index) => (
          <div key={index}>
            <dt>{item.label}</dt>
            <dd>
              <button type="button" onClick={() => jump(item)}>
                {item.value}
              </button>
            </dd>
          </div>
        ))}
      </dl>
    </>
  );
  const status = (
    <>
      {converted ? (
        <p className={formStyles.pfConversionNote}>
          저장 시 경로별 형식으로 전환됩니다. nginx 설정은 같습니다.
        </p>
      ) : null}
      {errors.length ? (
        <div className={formStyles.pfSummaryErrors} role="alert">
          <strong>입력 오류 {errors.length}개</strong>
          {errors.map((item, index) => (
            <button type="button" key={index} onClick={() => jump(item)}>
              {item.label} — {item.value}
            </button>
          ))}
        </div>
      ) : null}
    </>
  );
  const saveTarget =
    target?.kind === "node" ? (
      <p className={formStyles.pfSaveTarget}>
        저장 대상: {targetLabel(target)}
      </p>
    ) : null;
  const actions = (
    <div className={formStyles.pfSaveActions}>
      <Link
        href={cancelHref}
        className={buttonClassName({ variant: "secondary" })}
      >
        취소
      </Link>
      <Button
        type="submit"
        disabled={pending}
        aria-busy={pending || undefined}
        onClick={close}
      >
        {pending ? "저장 중..." : submitLabel}
      </Button>
    </div>
  );
  return (
    <>
      <aside
        className={formStyles.pfSummaryRail}
        ref={rail}
        aria-label="한눈에 보기"
      >
        <header>
          <h2>한눈에 보기</h2>
          {errors.length ? (
            <StatusBadge tone="error">오류 {errors.length}</StatusBadge>
          ) : null}
        </header>
        <div className={formStyles.pfSummaryBody}>{body}</div>
        <footer>
          {saveError}
          {status}
          <p className={formStyles.directiveGroupNote}>
            저장한 뒤 「설정 적용」으로 nginx에 반영합니다.
          </p>
          {saveTarget}
          {actions}
        </footer>
      </aside>
      <div className={formStyles.pfSaveBar} ref={bar}>
        {saveError}
        {converted ? (
          <span className={formStyles.pfConversionNote}>
            저장 시 경로별 형식으로 전환
          </span>
        ) : null}
        <Button type="button" variant="secondary" onClick={() => setOpen(true)}>
          한눈에 보기{errors.length ? ` · 오류 ${errors.length}` : ""}
        </Button>
        {saveTarget}
        {actions}
      </div>
      <dialog
        ref={sheet}
        className={formStyles.pfSummarySheet}
        onCancel={() => setOpen(false)}
        onClose={() => setOpen(false)}
        onClick={(event) => {
          if (event.target === event.currentTarget) setOpen(false);
        }}
        aria-labelledby="server-summary-title"
      >
        <header>
          <h2 id="server-summary-title">한눈에 보기</h2>
          <Button
            type="button"
            variant="ghost"
            onClick={() => setOpen(false)}
            aria-label="요약 닫기"
          >
            닫기
          </Button>
        </header>
        <div className={formStyles.pfSummaryBody}>
          {saveError}
          {body}
          {status}
        </div>
        <footer>
          {saveTarget}
          {actions}
        </footer>
      </dialog>
    </>
  );
}
