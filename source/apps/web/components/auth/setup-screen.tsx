import { AuthFrame } from "./auth-frame";
import { SetupForm } from "@/app/setup/setup-form";

/** Also rendered in Storybook; availability and first-run guards remain in the route. */
export function SetupScreen() {
  return <AuthFrame firstRun title="최초 관리자 설정" subtitle="최초 관리자 계정을 만들어 시작하세요."><SetupForm /></AuthFrame>;
}
