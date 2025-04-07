import { DocsLayout } from "@/components/docs-layout";
import { baseOptions } from "#/app/layout-config";
import { Navigation } from "@/components/nav";
import { Sidebar } from "#/components/docs-layout/sidebar";
import { openapiPages } from "./source";

export default async function Layout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <>
      <Navigation />
      <Sidebar>
        <DocsLayout isOpenApiSpec tree={openapiPages.pageTree} {...baseOptions}>
          {children}
        </DocsLayout>
      </Sidebar>
    </>
  );
}
