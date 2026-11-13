import { expect, it } from "vitest";
import { findReservedPortConflict, type ReservedPorts } from "./reserved-ports";
const tcp: ReservedPorts = { process: [3000, 3001, 5432], http: [80, 8080], streamTcp: [15555] };
it("only explicit UDP process reservations conflict with UDP streams", () => {
  const reserved = { ...tcp, processUdp: [5354], owners: { 5354: "이름 확인 서비스" } };
  expect(findReservedPortConflict(5354, "stream-udp", reserved)?.kind).toBe("process");
  expect(findReservedPortConflict(5354, "stream-udp", reserved)?.message).toContain("이름 확인 서비스");
  for (const port of [...tcp.process, ...tcp.http, ...tcp.streamTcp]) expect(findReservedPortConflict(port, "stream-udp", reserved)).toBeNull();
  for (const context of ["http", "stream-tcp"] as const) expect(findReservedPortConflict(5354, context, reserved)).toBeNull();
});
it("missing UDP reservations are empty, and TCP reservations retain their old behavior", () => {
  for (const reserved of [tcp, { ...tcp, processUdp: [] }]) {
    expect(findReservedPortConflict(5354, "stream-udp", reserved)).toBeNull();
    expect(findReservedPortConflict(3001, "stream-udp", reserved)).toBeNull();
    expect(findReservedPortConflict(3001, "stream-tcp", reserved)?.kind).toBe("process");
    expect(findReservedPortConflict(3001, "http", reserved)?.kind).toBe("process");
  }
});
