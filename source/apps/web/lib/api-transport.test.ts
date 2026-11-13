import { afterEach, describe, expect, it, vi } from "vitest";
import { safeLocalRedirect, submitApiForm } from "./api-transport";

afterEach(() => vi.unstubAllGlobals());

describe("API form boundary", () => {
  it("allows provider navigation only for the two OIDC start commands", async () => {
    const url = "https://identity.example.test/authorize?state=random&code_challenge=proof";
    vi.stubGlobal("fetch", async () => Response.json({ redirectTo: url }));
    for (const id of ["login.oauthSignIn", "account.startLink"]) {
      expect(await submitApiForm({ id, args: [] }, new FormData())).toEqual({ redirectTo: url });
    }
    expect((await submitApiForm<{ ok: boolean }>({ id: "account.unlink", args: [] }, new FormData())).result?.ok).toBe(false);
    for (const redirectTo of ["javascript:alert(1)", "//identity.example.test", "https://user:secret@identity.example.test/"]) {
      vi.stubGlobal("fetch", async () => Response.json({ redirectTo }));
      expect((await submitApiForm<{ ok: boolean }>({ id: "login.oauthSignIn", args: [] }, new FormData())).result?.ok).toBe(false);
    }
  });
  it("navigates to required MFA enrollment after session policy changes",async()=>{
    vi.stubGlobal("fetch",async()=>Response.json({error:{code:"two_factor_required",message:"enroll"}},{status:403}));
    expect(await submitApiForm({id:"nodes.pairNode",args:[]},new FormData())).toEqual({redirectTo:"/account/2fa"});
  });
  it("preserves repeated fields and replaces caller-supplied dispatch arguments", async () => {
    const original = new FormData();
    original.append("events", "issued");
    original.append("events", "failed");
    original.set("_apiArgs", '["injected"]');
    let requestBody: FormData | undefined;
    vi.stubGlobal("fetch", async (_url: string, init: RequestInit) => {
      requestBody = init.body as FormData;
      return Response.json({ result: { ok: true }, refresh: true });
    });
    const result = await submitApiForm({ id: "certificates.updateCertificate", args: ["cert-1"] }, original);
    expect(requestBody?.getAll("events")).toEqual(["issued", "failed"]);
    expect(requestBody?.get("_apiArgs")).toBe('["cert-1"]');
    expect(original.get("_apiArgs")).toBe('["injected"]');
    expect(result).toEqual({ result: { ok: true }, refresh: true });
  });

  it("turns non-JSON upstream failures into a safe form error", async () => {
    vi.stubGlobal("fetch", async () => new Response("database password must never be displayed", { status: 502 }));
    const result = await submitApiForm({ id: "certificates.createCertificate", args: [] }, new FormData());
    expect(result.result).toMatchObject({ ok: false });
    expect(JSON.stringify(result)).not.toContain("database password");
  });

  it("rejects unsafe navigation while keeping local paths and query strings", () => {
    expect(safeLocalRedirect("/certificates/a?updated=1")).toBe("/certificates/a?updated=1");
    for (const target of ["https://evil.test/", "//evil.test/", "/\\evil.test/", "javascript:alert(1)", "/a\nLocation: evil", "/%2f%2fevil.test", "/%5cevil.test"]) {
      expect(safeLocalRedirect(target)).toBeNull();
    }
  });
});
