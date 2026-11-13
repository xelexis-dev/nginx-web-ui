import type { components } from "@repo/contracts";
import { formatDateTime } from "@/app/(app)/account/tokens/tokens";

export type PasskeyList = components["schemas"]["PasskeyList"];
export type PasskeyDetail = components["schemas"]["PasskeyDetail"];
export type PasskeyRow = components["schemas"]["PasskeyRow"];
export type PasskeyRowView = Omit<PasskeyRow, "lastUsedAt"> & {
  address: string;
  lastUsedAt: string;
};
export type PasskeyListView = Omit<PasskeyList, "passkeys"> & {
  passkeys: PasskeyRowView[];
  currentAddress: string | null;
};

/** Server pages format dates once; browser components receive strings, never local Date values. */
export function passkeyRowView(row: PasskeyRow): PasskeyRowView {
  return {
    ...row,
    address: new URL(row.origin).host,
    createdAt: formatDateTime(new Date(row.createdAt), "—"),
    lastUsedAt: formatDateTime(
      row.lastUsedAt ? new Date(row.lastUsedAt) : null,
      "아직 사용하지 않음",
    ),
  };
}
export function passkeyListView(data: PasskeyList): PasskeyListView {
  return {
    ...data,
    passkeys: data.passkeys.map(passkeyRowView),
    currentAddress: data.currentOrigin
      ? new URL(data.currentOrigin).host
      : null,
  };
}
