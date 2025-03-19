import { DocsLayout } from "@/components/docs-layout";
import { repoDocsPages } from "@/app/source";
import { baseOptions } from "../layout-config";
import { Navigation } from "@/components/nav";
import { RedirectsHandler } from "./redirects-handler";
import { Sidebar } from "#/components/docs-layout/sidebar";

export default async function Layout({
  children,
  params,
}: {
  children: React.ReactNode;
  params: Promise<{ slug?: string[] }>;
}) {
  const { slug } = await params;
  return (
    <>
      <Navigation />
      <Sidebar>
        <DocsLayout tree={repoDocsPages.pageTree} path={slug} {...baseOptions}>
          {children}
        </DocsLayout>
      </Sidebar>
      <RedirectsHandler />
    </>
  );
}
