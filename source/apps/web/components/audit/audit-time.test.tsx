import { expect,it,vi } from "vitest";
vi.mock("next/navigation", () => ({ useRouter: () => ({ push: vi.fn() }) }));
import { formatAuditTime } from "./audit-list";
it("labels audit and overview timestamps in explicit UTC",()=>{
  const date=new Date("2026-10-25T01:00:00Z");
  const expected=new Intl.DateTimeFormat("ko-KR",{dateStyle:"medium",timeStyle:"medium",timeZone:"UTC"}).format(date)+" UTC";
  expect(formatAuditTime(date)).toBe(expected);
});

import { renderToStaticMarkup } from "react-dom/server";
import { AuditList, formatAuditActor } from "./audit-list";
import { AuditDetail } from "./audit-detail";
it("renders escaped token provenance in local and node audit views", () => {
  const actor = { actorEmail: "owner@example.test", actorLabel: "unchanged", via: {kind:"token" as const,id:"id",name:'<script>alert("x")</script>'} };
  expect(formatAuditActor(actor)).toBe('owner@example.test · 토큰 <script>alert("x")</script>');
  expect(formatAuditActor({...actor,via:null})).toBe("owner@example.test");
  for (const basePath of ["/audit", "/nodes/n/audit"]) {
    const list=renderToStaticMarkup(<AuditList basePath={basePath} data={{items:[{id:"log",time:"UTC",action:"config:write",target:null,...actor}],total:1,page:1,totalPages:1,actions:[],actionFilter:""}}/>);
    const detail=renderToStaticMarkup(<AuditDetail basePath={basePath} data={{action:"config:write",time:"UTC",target:null,ip:null,beforeJson:null,afterJson:null,...actor}}/>);
    for (const html of [list,detail]) {
      expect(html).toContain("owner@example.test · 토큰 &lt;script&gt;");
      expect(html).not.toContain("<script>");
    }
  }
});
