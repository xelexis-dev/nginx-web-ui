"use client";

import { useEffect, useState } from "react";
import {
  API_UNAVAILABLE_BODY,
  API_UNAVAILABLE_CSS,
  API_UNAVAILABLE_NOTE,
  API_UNAVAILABLE_TITLE,
  RETRY_SECONDS,
} from "@/lib/api-unavailable";

/**
 * The same screen the middleware serves as a plain document, as a React tree — for the pages the
 * middleware does not cover (`/login`, `/setup`), whose own API read can fail. One stylesheet and
 * one set of words for both, so a person sees the same thing wherever the API went missing.
 */
export function ApiUnavailableScreen() {
  const [left, setLeft] = useState(RETRY_SECONDS);
  useEffect(() => {
    const timer = setInterval(() => {
      setLeft((value) => {
        if (value <= 1) window.location.reload();
        return value - 1;
      });
    }, 1000);
    return () => clearInterval(timer);
  }, []);

  return (
    <>
      <style>{API_UNAVAILABLE_CSS}</style>
      <div className="nwui-unavailable" role="alert">
        <p className="brand">nginx-web-ui</p>
        {/* No status line here: a page cannot set the response status, and the document version
            that really answers 503 is the one entitled to say so. */}
        <div className="rule" />
        <h1>{API_UNAVAILABLE_TITLE}</h1>
        <p>{API_UNAVAILABLE_BODY}</p>
        <p>{API_UNAVAILABLE_NOTE}</p>
        <div className="actions">
          <button type="button" onClick={() => window.location.reload()}>
            다시 시도
          </button>
        </div>
        <p className="countdown">{left > 0 ? left : 0}초 뒤 자동으로 다시 시도합니다.</p>
      </div>
    </>
  );
}
