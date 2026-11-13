export interface AccountState {
  ok: boolean;
  error?: string;
  fieldErrors?: Record<string, string>;
}
export interface TwoFactorState {
  ok: boolean;
  error?: string;
  backupCodes?: string[];
}
