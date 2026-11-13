import { beforeEach, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { isValidElement, type ReactElement, type ReactNode } from "react";
const mocks=vi.hoisted(()=>({read:vi.fn()}));
vi.mock("@/lib/api-page",()=>({readPageApi:mocks.read}));
vi.mock("@repo/db",()=>({getDb:()=>{throw new Error("SSR must use API");},getWebhookTarget:vi.fn(),listWebhookTargets:vi.fn(),getMailRecipient:vi.fn()}));
vi.mock("@/lib/guard",()=>({requireRole:()=>{throw new Error("API owns authorization");}}));
vi.mock("@/lib/webhook-dispatch",()=>({openTarget:()=>{throw new Error("SSR must not decrypt");}}));
vi.mock("./webhook-form",()=>({WebhookForm:()=>null}));
vi.mock("./[id]/detail-actions",()=>({WebhookDetailActions:()=>null}));
vi.mock("../mail/[id]/detail-actions",()=>({MailRecipientDetailActions:()=>null}));
vi.mock("../mail/recipient-form",()=>({MailRecipientForm:()=>null}));
vi.mock("../../settings/smtp/smtp-form",()=>({SmtpForm:()=>null}));
const target={id:"w1",name:"Ops",enabled:true,format:"custom",events:["cert.issued"],lastAttemptAt:"2026-09-22T12:00:00.000Z",lastSuccessAt:null,lastStatus:503,lastError:"HTTP 503",bodyTemplate:"{}",maskedUrl:"https://hooks.example.test/api/...",headerNames:["Authorization"],openError:null};
function element<T>(node:ReactNode,type:unknown):ReactElement<T>{
 const visit=(n:ReactNode):ReactElement<T>|undefined=>{if(Array.isArray(n))return n.map(visit).find(Boolean);if(!isValidElement(n))return undefined;if(n.type===type)return n as ReactElement<T>;return visit((n.props as {children?:ReactNode}).children);};const found=visit(node);expect(found).toBeDefined();return found!;
}
beforeEach(()=>{mocks.read.mockReset();mocks.read.mockResolvedValue(target);});
it("webhook detail renders API-masked metadata and ISO timestamps",async()=>{
 const Page=(await import("./[id]/page")).default;const html=renderToStaticMarkup(await Page({params:Promise.resolve({id:"w1"})}));
 expect(mocks.read).toHaveBeenCalledWith("/api/ui/v1/admin/notifications/webhooks/w1");expect(html).toContain(target.maskedUrl);expect(html).toContain("Authorization: ******");expect(html).toContain("HTTP 503");expect(html).toContain((new Date(target.lastAttemptAt).toLocaleString("ko-KR", { dateStyle: "medium", timeStyle: "medium", timeZone: "UTC" }) + " UTC"));
});
it("webhook edit passes only safe defaults and secret placeholders",async()=>{
 const Page=(await import("./[id]/edit/page")).default;const{WebhookForm}=await import("./webhook-form");
 const props=element<{maskedUrl:string;storedHeaderNames:string[];defaults:Record<string,unknown>}>(await Page({params:Promise.resolve({id:"w1"})}),WebhookForm).props;
 expect(props.maskedUrl).toBe(target.maskedUrl);expect(props.storedHeaderNames).toEqual(["Authorization"]);expect(props.defaults).toEqual({id:"w1",name:"Ops",enabled:true,format:"custom",events:["cert.issued"],bodyTemplate:"{}"});
});
it("webhook list uses API rows and preserves empty and failed states",async()=>{
 const Page=(await import("./page")).default;mocks.read.mockResolvedValue([target]);const html=renderToStaticMarkup(await Page());expect(html).toContain("Ops");expect(html).toContain("실패");mocks.read.mockResolvedValue([]);expect(renderToStaticMarkup(await Page())).toContain("등록된 webhook 이 없습니다");
});
it("webhook creation obtains API authorization before rendering static defaults",async()=>{
 const Page=(await import("./new/page")).default;mocks.read.mockResolvedValue([]);await Page();expect(mocks.read).toHaveBeenCalledWith("/api/ui/v1/admin/notifications/webhooks");
});
it("mail detail reads safe API row and formats timestamps",async()=>{
 const Page=(await import("../mail/[id]/page")).default;mocks.read.mockResolvedValue({id:"m1",label:"Ops",address:"ops@example.test",enabled:true,events:["cert.issued"],lastAttemptAt:target.lastAttemptAt,lastSuccessAt:null,lastError:null});
 const html=renderToStaticMarkup(await Page({params:Promise.resolve({id:"m1"})}));expect(mocks.read).toHaveBeenCalledWith("/api/ui/v1/admin/notifications/mail/m1");expect(html).toContain("ops@example.test");expect(html).toContain((new Date(target.lastAttemptAt).toLocaleString("ko-KR", { dateStyle: "medium", timeStyle: "medium", timeZone: "UTC" }) + " UTC"));
});

it("webhook edit preserves a safe decryption error without reconstructing credentials",async()=>{
 const Page=(await import("./[id]/edit/page")).default;
 mocks.read.mockResolvedValue({...target,maskedUrl:null,headerNames:[],openError:"저장된 비밀 값을 읽을 수 없습니다."});
 const html=renderToStaticMarkup(await Page({params:Promise.resolve({id:"w1"})}));
 expect(html).toContain("저장된 비밀 값을 읽을 수 없습니다.");
 expect(html).not.toContain(target.maskedUrl);
});

it("API not-found and authentication redirects propagate instead of rendering empty detail",async()=>{
 const Page=(await import("./[id]/page")).default;
 const redirect=new Error("NEXT_REDIRECT");mocks.read.mockRejectedValue(redirect);
 await expect(Page({params:Promise.resolve({id:"missing"})})).rejects.toBe(redirect);
});

const smtp = { config: { enabled: true, host: "smtp.example.test", port: 587, secure: false, username: "ops", fromEmail: "nginx@example.test", fromName: "Nginx", rejectUnauthorized: true }, hasStoredPassword: true, readiness: { ok: true } };
const recipient = { id: "m1", label: "Ops", address: "ops@example.test", enabled: true, events: ["cert.issued"], lastAttemptAt: target.lastAttemptAt, lastSuccessAt: null, lastError: "SMTP 연결 실패" };

it("mail list reads recipients and safe readiness from the API",async()=>{
 const Page=(await import("../mail/page")).default;
 mocks.read.mockImplementation(async (path:string)=>path.endsWith("/smtp")?{...smtp,readiness:{ok:false,reason:"disabled",message:"SMTP가 꺼져 있습니다."}}:[recipient]);
 const html=renderToStaticMarkup(await Page());expect(html).toContain("SMTP가 꺼져 있습니다.");expect(html).toContain("ops@example.test");expect(html).toContain((new Date(target.lastAttemptAt).toLocaleString("ko-KR", { dateStyle: "medium", timeStyle: "medium", timeZone: "UTC" }) + " UTC"));
 expect(mocks.read.mock.calls.map(call=>call[0])).toEqual(["/api/ui/v1/admin/notifications/mail","/api/ui/v1/admin/settings/smtp"]);
});

it("new mail recipient preserves the not-configured warning from safe SMTP readiness",async()=>{
 const Page=(await import("../mail/new/page")).default;const{MailRecipientForm}=await import("../mail/recipient-form");
 mocks.read.mockResolvedValue({...smtp,hasStoredPassword:false,readiness:{ok:false,reason:"not-configured",message:"SMTP가 설정되지 않았습니다."}});
 const props=element<{smtpReady:boolean;smtpProblem:string}>(await Page(),MailRecipientForm).props;
 expect(props.smtpReady).toBe(false);expect(props.smtpProblem).toBe("SMTP가 설정되지 않았습니다.");
});

it("mail edit gets its row and exposes the password-unreadable readiness without any secret",async()=>{
 const Page=(await import("../mail/[id]/edit/page")).default;const{MailRecipientForm}=await import("../mail/recipient-form");
 mocks.read.mockImplementation(async (path:string)=>path.endsWith("/smtp")?{...smtp,readiness:{ok:false,reason:"password-unreadable",message:"비밀번호를 다시 입력하세요."}}:recipient);
 const props=element<{defaults:Record<string,unknown>;smtpReady:boolean;smtpProblem:string}>(await Page({params:Promise.resolve({id:"m1"})}),MailRecipientForm).props;
 expect(props.defaults).toEqual({id:"m1",label:"Ops",address:"ops@example.test",enabled:true,events:["cert.issued"]});expect(props.smtpReady).toBe(false);expect(props.smtpProblem).toBe("비밀번호를 다시 입력하세요.");
});

it.each([true,false])("SMTP passes only safe config defaults and stored-password presence (%s)",async stored=>{
 const Page=(await import("../../settings/smtp/page")).default;const{SmtpForm}=await import("../../settings/smtp/smtp-form");
 const config=stored?smtp.config:{enabled:false,host:"",port:587,secure:false,username:"",fromEmail:"",fromName:"",rejectUnauthorized:true};
 mocks.read.mockResolvedValue({...smtp,config,hasStoredPassword:stored});
 const props=element<{defaults:typeof config;hasStoredPassword:boolean}>(await Page(),SmtpForm).props;
 expect(props).toEqual({defaults:config,hasStoredPassword:stored});expect(mocks.read).toHaveBeenCalledWith("/api/ui/v1/admin/settings/smtp");
});

it("SMTP renders a safe password recovery error",async()=>{
 const Page=(await import("../../settings/smtp/page")).default;
 mocks.read.mockResolvedValue({...smtp,readiness:{ok:false,reason:"password-unreadable",message:"비밀번호를 다시 입력하세요."}});
 expect(renderToStaticMarkup(await Page())).toContain("비밀번호를 다시 입력하세요.");
});
