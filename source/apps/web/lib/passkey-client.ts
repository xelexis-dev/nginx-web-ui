"use client";

import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import type { BasicFormState, FormResult } from "@repo/contracts";
import { submitApiForm } from "./api-transport";

export type Ceremony = "sign-in" | "registration";
export type PasskeyPhase =
  "idle" | "waiting" | "cancelled" | "failed" | "duplicate";
type Result = Partial<BasicFormState> & { publicKey?: Record<string, unknown> };

export function supportsPasskeys(): boolean {
  return (
    typeof PublicKeyCredential !== "undefined" &&
    typeof PublicKeyCredential.parseCreationOptionsFromJSON === "function" &&
    typeof PublicKeyCredential.parseRequestOptionsFromJSON === "function" &&
    typeof PublicKeyCredential.prototype.toJSON === "function" &&
    typeof navigator !== "undefined" &&
    !!navigator.credentials
  );
}
const subscribe = () => () => {};
export function usePasskeySupport(): boolean {
  return useSyncExternalStore(subscribe, supportsPasskeys, () => false);
}

export function passkeyFailure(
  error: unknown,
  ceremony: Ceremony,
): PasskeyPhase {
  const name = error instanceof Error ? error.name : "";
  if (name === "NotAllowedError" || name === "AbortError") return "cancelled";
  return name === "InvalidStateError" && ceremony === "registration"
    ? "duplicate"
    : "failed";
}

/** Browser-only ceremonies; the API owns challenges, proof, validation and session cookies. */
export async function runPasskey(
  ceremony: Ceremony,
  form: FormData,
  signal: AbortSignal,
): Promise<FormResult<Result>> {
  const registration = ceremony === "registration";
  const start = await submitApiForm<Result>(
    {
      id: registration ? "account.passkeys.start" : "login.passkeyStart",
      args: [],
    },
    form,
  );
  if (start.redirectTo) {
    window.location.assign(start.redirectTo);
    return start;
  }
  signal.throwIfAborted();
  if (start.result?.ok === false) return start;
  if (!start.result?.publicKey)
    return { result: { ...start.result, ok: false } };
  const options = start.result.publicKey;
  const credential = registration
    ? await navigator.credentials.create({
        publicKey: PublicKeyCredential.parseCreationOptionsFromJSON(
          options as unknown as PublicKeyCredentialCreationOptionsJSON,
        ),
        signal,
      })
    : await navigator.credentials.get({
        publicKey: PublicKeyCredential.parseRequestOptionsFromJSON(
          options as unknown as PublicKeyCredentialRequestOptionsJSON,
        ),
        signal,
      });
  signal.throwIfAborted();
  if (!(credential instanceof PublicKeyCredential))
    throw new Error("Missing public key credential");
  const finish = new FormData();
  finish.set("credential", JSON.stringify(credential.toJSON()));
  const response = await submitApiForm<Result>(
    {
      id: registration ? "account.passkeys.finish" : "login.passkeyFinish",
      args: [],
    },
    finish,
  );
  if (response.redirectTo) window.location.assign(response.redirectTo);
  return response;
}

export function usePasskeyCeremony(
  ceremony: Ceremony,
  initial: PasskeyPhase = "idle",
) {
  const supported = usePasskeySupport();
  const [phase, setPhase] = useState<PasskeyPhase>(initial);
  const [result, setResult] = useState<Result>({});
  const controller = useRef<AbortController | null>(null);
  useEffect(() => () => controller.current?.abort(), []);
  async function submit(form: FormData) {
    if (phase === "waiting") return;
    controller.current = new AbortController();
    setResult({});
    setPhase("waiting");
    try {
      const response = await runPasskey(
        ceremony,
        form,
        controller.current.signal,
      );
      setResult(response.result ?? {});
      setPhase(
        response.result?.ok === false || response.result?.error
          ? "failed"
          : "idle",
      );
    } catch (error) {
      setPhase(passkeyFailure(error, ceremony));
    }
  }
  return {
    supported,
    phase,
    result,
    submit,
    cancel: () => controller.current?.abort(),
  };
}
