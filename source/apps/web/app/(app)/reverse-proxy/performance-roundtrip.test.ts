import { describe, expect, it } from "vitest";
import { toServerFormValues } from "@/components/reverse-proxy/server-form-vm";

// API persistence/rendering is covered by Rust control_resources/control_golden.
// These assertions retain the frontend edit presentation boundary.
const row = (config: unknown) => ({ id: "c43763cc-7f68-4e86-9937-3a78c091ea6b", serverName: "cache.example.test", listen: 80, enabled: true, upstreamId: null, certificateId: null, config });
describe("API performance settings → edit values", () => {
  it("keeps the server gzip setting unset when only a location is enabled", () => {
    const values = toServerFormValues(row({ locations: [{ path: "/", proxyPass: "http://origin.test", gzipEnabled: true }] }), []);
    expect(values.gzip).toBeUndefined();
    expect(values.locations?.[0]?.gzipEnabled).toBe(true);
  });
  it("retains server settings returned by the API", () => {
    const values = toServerFormValues(row({ proxyPass: "http://origin.test", cache: { enabled: true, valid: "1h", maxSize: "500m" }, gzip: { enabled: true, compLevel: 7, minLength: 2048 } }), []);
    expect(values).toMatchObject({ cache: { enabled: true, valid: "1h", maxSize: "500m" }, gzip: { enabled: true, compLevel: 7, minLength: 2048 } });
  });
  it("retains location overrides and resolves a stored upstream when opening edit", () => {
    const values = toServerFormValues(row({cache:{enabled:false},gzip:{enabled:false},locations:[
      {path:"/static",proxyPass:"http://origin-pool",cacheEnabled:true,gzipEnabled:true},
      {path:"/stream",proxyPass:"http://origin.test",cacheEnabled:false,gzipEnabled:false},
      {path:"/",proxyPass:"http://origin.test"},
    ]}), [{id:"u1",name:"origin-pool"}]);
    expect(values).toMatchObject({cache:{enabled:false},gzip:{enabled:false}});
    expect(values.locations?.[0]).toMatchObject({upstreamId:"u1",cacheEnabled:true,gzipEnabled:true});
    expect(values.locations?.[1]).toMatchObject({cacheEnabled:false,gzipEnabled:false});
    expect(values.locations?.[2]?.cacheEnabled).toBeUndefined();
  });
});
