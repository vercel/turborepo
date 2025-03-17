import { DocsLayout } from "@/components/docs-layout";
import { repoDocsPages } from "@/app/source";
import { baseOptions } from "../layout-config";
import { Navigation } from "@/components/nav";
import { RedirectsHandler } from "./redirects-handler";
import { Sidebar } from "#/components/docs-layout/sidebar";

export default function Layout({ children }: { children: React.ReactNode }) {
  return (
    <>
      <Navigation />
      <Sidebar>
        <DocsLayout tree={repoDocsPages.pageTree} {...baseOptions}>
          {children}
        </DocsLayout>
      </Sidebar>
      <RedirectsHandler />
    </>
  );
}
