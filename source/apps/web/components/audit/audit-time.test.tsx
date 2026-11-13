import { expect,it } from "vitest";
import { formatAuditTime } from "./audit-list";
it("labels audit and overview timestamps in explicit UTC",()=>{
  const date=new Date("2026-10-25T01:00:00Z");
  const expected=new Intl.DateTimeFormat("ko-KR",{dateStyle:"medium",timeStyle:"medium",timeZone:"UTC"}).format(date)+" UTC";
  expect(formatAuditTime(date)).toBe(expected);
});
