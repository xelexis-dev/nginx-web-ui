"use client";

import { useState } from "react";
import { Banner, Button } from "@repo/ui";
import type { DirectiveEditorState } from "@repo/presentation/nginx/directive-state";
import { applyWebSocketPreset, type WebSocketPresetResult, type WebSocketPresetResolution } from "@repo/presentation/nginx/websocket-preset";
import styles from "./websocket-preset.module.css";

export function WebSocketPresetFeedback({ result, onReplace, onDismiss }: {
  result: WebSocketPresetResult;
  onReplace: () => void;
  onDismiss: () => void;
}) {
  if (result.status === "applied") return null;
  if (result.status === "invalid") return <Banner tone="error" role="alert" title="고급 디렉티브를 먼저 확인하세요">{result.error}</Banner>;
  return <Banner tone="warning" role="status" title="기존 WebSocket 설정과 충돌합니다">
    <p>다음 설정을 교체할지 선택하세요. 다른 헤더와 시간 제한은 유지됩니다.</p>
    <ul>{result.conflicts.map(conflict => <li key={conflict.field}>
      {conflict.field === "httpVersion" ? "HTTP 버전" : conflict.field}: {conflict.existingValues.map(value => value || "(빈 값)").join(", ")} → {conflict.expectedValue}
    </li>)}</ul>
    <div className={styles.actions}>
      <Button type="button" variant="secondary" onClick={onReplace}>충돌한 설정 교체</Button>
      <Button type="button" variant="ghost" onClick={onDismiss}>기존 설정 유지</Button>
    </div>
  </Banner>;
}

export function WebSocketPresetControl({ value, onChange }: { value: DirectiveEditorState; onChange: (state: DirectiveEditorState) => void }) {
  const [requested, setRequested] = useState(false);
  // Derive conflicts from current edits, never from an old snapshot waiting for confirmation.
  const result = requested ? applyWebSocketPreset(value) : null;
  function apply(resolution?: WebSocketPresetResolution) {
    const next = applyWebSocketPreset(value, resolution);
    if (next.status === "applied") { onChange(next.state); setRequested(false); }
    else setRequested(true);
  }
  return <div className={styles.control}>
    <div><Button type="button" variant="secondary" onClick={() => apply()}>WebSocket 헤더 채우기</Button></div>
    {result ? <WebSocketPresetFeedback result={result} onReplace={() => apply("replace")} onDismiss={() => setRequested(false)} /> : null}
  </div>;
}
