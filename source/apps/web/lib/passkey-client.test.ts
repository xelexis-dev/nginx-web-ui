import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({
  submit: vi.fn(),
  assign: vi.fn(),
  create: vi.fn(),
  get: vi.fn(),
  parseCreate: vi.fn(),
  parseGet: vi.fn(),
}));
vi.mock("./api-transport", () => ({ submitApiForm: mocks.submit }));
import { passkeyFailure, runPasskey, supportsPasskeys } from "./passkey-client";

beforeEach(() => {
  vi.clearAllMocks();
  class Credential {
    static parseCreationOptionsFromJSON = mocks.parseCreate;
    static parseRequestOptionsFromJSON = mocks.parseGet;
    toJSON() {
      return { id: "synthetic", response: { clientDataJSON: "browser-json" } };
    }
  }
  vi.stubGlobal("PublicKeyCredential", Credential);
  vi.stubGlobal("navigator", {
    credentials: { create: mocks.create, get: mocks.get },
  });
  vi.stubGlobal("window", { location: { assign: mocks.assign } });
  mocks.create.mockResolvedValue(new Credential());
  mocks.get.mockResolvedValue(new Credential());
  mocks.parseCreate.mockReturnValue({ challenge: "parsed-create" });
  mocks.parseGet.mockReturnValue({ challenge: "parsed-get" });
});
afterEach(() => vi.unstubAllGlobals());

it("requires native JSON parsers, toJSON and the credential API", () => {
  expect(supportsPasskeys()).toBe(true);
  for (const member of [
    "parseCreationOptionsFromJSON",
    "parseRequestOptionsFromJSON",
  ]) {
    const value = Object.getOwnPropertyDescriptor(PublicKeyCredential, member)!;
    Object.defineProperty(PublicKeyCredential, member, {
      value: undefined,
      configurable: true,
    });
    expect(supportsPasskeys()).toBe(false);
    Object.defineProperty(PublicKeyCredential, member, value);
  }
  vi.stubGlobal("PublicKeyCredential", undefined);
  expect(supportsPasskeys()).toBe(false);
});

it.each(["sign-in", "registration"] as const)(
  "uses native %s options and serializes only the credential for finish",
  async (ceremony) => {
    mocks.submit
      .mockResolvedValueOnce({
        result: { publicKey: { challenge: "encoded" } },
      })
      .mockResolvedValueOnce({ redirectTo: "/account/passkeys/key?added=1" });
    const form = new FormData();
    form.set("currentPassword", "synthetic-password");
    const signal = new AbortController().signal;
    await runPasskey(ceremony, form, signal);
    expect(mocks.submit.mock.calls[0]![0]).toEqual({
      id:
        ceremony === "registration"
          ? "account.passkeys.start"
          : "login.passkeyStart",
      args: [],
    });
    expect(
      ceremony === "registration" ? mocks.create : mocks.get,
    ).toHaveBeenCalledWith({
      publicKey: {
        challenge: ceremony === "registration" ? "parsed-create" : "parsed-get",
      },
      signal,
    });
    const finish = mocks.submit.mock.calls[1]!;
    expect(finish[0]).toEqual({
      id:
        ceremony === "registration"
          ? "account.passkeys.finish"
          : "login.passkeyFinish",
      args: [],
    });
    expect([...finish[1].keys()]).toEqual(["credential"]);
    expect(JSON.parse(finish[1].get("credential"))).toEqual({
      id: "synthetic",
      response: { clientDataJSON: "browser-json" },
    });
    expect(mocks.assign).toHaveBeenCalledExactlyOnceWith(
      "/account/passkeys/key?added=1",
    );
  },
);

it.each([
  { result: { ok: false } },
  { result: { ok: false, publicKey: {} } },
  { result: { ok: false, error: "network" } },
  { result: { ok: false, fieldErrors: { currentPassword: "wrong" } } },
])(
  "never opens a prompt after a returned start failure %o",
  async (failure) => {
    mocks.submit.mockResolvedValue(failure);
    expect(
      await runPasskey("sign-in", new FormData(), new AbortController().signal),
    ).toEqual(failure);
    expect(mocks.get).not.toHaveBeenCalled();
    expect(mocks.create).not.toHaveBeenCalled();
    expect(mocks.submit).toHaveBeenCalledTimes(1);
  },
);

it("navigates an API redirect before opening a prompt", async () => {
  mocks.submit.mockResolvedValue({ redirectTo: "/login" });
  await runPasskey(
    "registration",
    new FormData(),
    new AbortController().signal,
  );
  expect(mocks.assign).toHaveBeenCalledWith("/login");
  expect(mocks.create).not.toHaveBeenCalled();
});
it("own cancellation aborts the native prompt and never submits finish", async () => {
  mocks.submit.mockResolvedValue({ result: { publicKey: {} } });
  mocks.get.mockImplementation(
    ({ signal }) =>
      new Promise((_resolve, reject) =>
        signal.addEventListener("abort", () =>
          reject(new DOMException("cancel", "AbortError")),
        ),
      ),
  );
  const controller = new AbortController();
  const result = runPasskey("sign-in", new FormData(), controller.signal);
  await vi.waitFor(() => expect(mocks.get).toHaveBeenCalled());
  controller.abort();
  await expect(result).rejects.toHaveProperty("name", "AbortError");
  expect(mocks.submit).toHaveBeenCalledTimes(1);
});
it("does not open a prompt when cancelled during the start request", async () => {
  mocks.submit.mockResolvedValue({ result: { publicKey: {} } });
  const controller = new AbortController();
  controller.abort();
  await expect(
    runPasskey("registration", new FormData(), controller.signal),
  ).rejects.toHaveProperty("name", "AbortError");
  expect(mocks.create).not.toHaveBeenCalled();
});
describe("prompt outcomes", () => {
  it.each(["NotAllowedError", "AbortError"])("%s means cancelled", (name) =>
    expect(passkeyFailure(new DOMException("", name), "registration")).toBe(
      "cancelled",
    ),
  );
  it("uses the duplicate line only for registration", () => {
    const error = new DOMException("", "InvalidStateError");
    expect(passkeyFailure(error, "registration")).toBe("duplicate");
    expect(passkeyFailure(error, "sign-in")).toBe("failed");
  });
  it("all other errors mean failed", () =>
    expect(passkeyFailure(new Error("bad"), "registration")).toBe("failed"));
});

it("does not finish when the credential API returns no credential", async () => {
  mocks.submit.mockResolvedValue({ result: { publicKey: {} } });
  mocks.get.mockResolvedValue(null);
  await expect(runPasskey("sign-in", new FormData(), new AbortController().signal)).rejects.toThrow("Missing public key credential");
  expect(mocks.submit).toHaveBeenCalledTimes(1);
});

it("requires native toJSON as well as the parsers", () => {
  Object.defineProperty(PublicKeyCredential.prototype, "toJSON", { value: undefined });
  expect(supportsPasskeys()).toBe(false);
});
