import { readSession } from "@/lib/api-client";
import { readPageApi } from "@/lib/api-page";
import { passkeyListView, type PasskeyList } from "@/lib/passkey-api";
import { NewPasskeyView } from "../new-passkey";
export const runtime = "nodejs";
export default async function NewPasskeyPage() {
  const data = passkeyListView(
    await readPageApi<PasskeyList>("/api/ui/v1/account/passkeys"),
  );
  const session = await readSession();
  return <NewPasskeyView data={data} role={session.user.role} />;
}
