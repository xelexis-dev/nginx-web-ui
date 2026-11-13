import type { Meta, StoryObj } from "@storybook/react-vite";
import { PageHeader, Facts } from "@repo/ui";
import { StreamForm } from "@/components/streams/stream-form";
import { UpstreamForm } from "@/components/upstreams/upstream-form";
import { CertificateEditForm } from "@/components/certificates/certificate-edit-form";
import { ProviderForm } from "@/app/admin/oauth/provider-form";
import { NodeEditForm } from "@/app/(node)/nodes/[id]/edit/node-edit-form";
import { CancelIssuanceButton } from "@/components/certificates/cancel-button";
import { NodeErrorCard } from "@/components/node-error";
import { formatAuditTime } from "@/components/audit/audit-list";
import type { NodeActionState } from "@/lib/node-action-state";
import { productFrame, productViewports } from "./frame";
import layout from "@/components/page-layout.module.css";

async function reject(_state:NodeActionState,form:FormData):Promise<NodeActionState>{
  return {ok:false,error:"입력값을 확인하세요. 저장하지 않았습니다.",fieldErrors:Object.fromEntries([...form.keys()].filter(k=>k!=="id").map(k=>[k,"입력값을 확인하세요."]))};
}
function AuditForms({kind}:{kind:"stream"|"upstream"|"provider"|"provider-edit"|"node"|"certificate"|"certificate-stored"|"errors"}){
  return <section className={layout.page}><PageHeader title="오류 후 입력 확인"/>
    {kind==="stream"?<StreamForm action={reject} cancelHref="/stream" defaultValues={{id:"s1",listen:15432,protocol:"tcp",proxyPass:"db.example.test:5432"}}/>:null}
    {kind==="upstream"?<UpstreamForm action={reject} cancelHref="/reverse-proxy/upstreams" defaultValues={{id:"u1",name:"backend",servers:["app.example.test:9000"]}}/>:null}
    {kind==="provider"?<ProviderForm mode="create"/>:null}
    {kind==="provider-edit"?<ProviderForm mode="edit" defaults={{id:"oidc",name:"OIDC",issuer:"https://issuer.example.test",clientId:"client",scope:"openid email profile",sortOrder:0,enabled:true}}/>:null}
    {kind==="node"?<NodeEditForm id="n1" defaults={{name:"운영 노드",baseUrl:"https://node.example.test",allowInsecureTls:false}} tokenRole="admin" registeredGatewayName="게이트웨이"/>:null}
    {kind.startsWith("certificate")?<CertificateEditForm id="c1" action={reject} cancelHref="/certificates/c1" defaults={{domains:["app.example.test"],alias:"운영 인증서",email:"ops@example.test",staging:false,challengeType:"http-01",dnsProvider:""}} isIssued hasStoredToken={kind==="certificate-stored"}/>:null}
    {kind==="errors"?<><NodeErrorCard failure={{kind:"node_error",message:"정해진 시간 안에 응답이 없습니다. 목록에서 결과를 먼저 확인하세요."}} nodeId="n1" retryHref="/nodes/n1/history" listHref="/nodes/n1/history"/><CancelIssuanceButton cancelPath="/__preview__/cancel"/><Facts items={[{term:"마지막 적용",value:formatAuditTime(new Date("2026-10-25T01:00:00Z"))}]}/></>:null}
  </section>;
}
const meta={id:"product-audit-forms",title:"Product/오류 후 입력 유지",component:AuditForms,tags:["!autodocs"],parameters:{layout:"fullscreen",viewport:{options:productViewports}},decorators:[productFrame(()=>({path:"/stream/s1/edit",label:"입력 확인",role:"admin"}))],args:{kind:"stream"}} satisfies Meta<typeof AuditForms>;
export default meta;
type Story=StoryObj<typeof meta>;
export const Stream:Story={};
export const Upstream:Story={args:{kind:"upstream"}};
export const Provider:Story={args:{kind:"provider"}};
export const Node:Story={args:{kind:"node"}};
export const Certificate:Story={args:{kind:"certificate"}};
export const Errors:Story={args:{kind:"errors"}};

export const ProviderEdit:Story={args:{kind:"provider-edit"}};
export const CertificateStored:Story={args:{kind:"certificate-stored"}};
