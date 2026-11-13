import { renderBreadcrumbs, type BreadcrumbPageProps } from "@/lib/breadcrumbs";
export default function Page(props: BreadcrumbPageProps) {
  return renderBreadcrumbs("/account/passkeys/new", props);
}
