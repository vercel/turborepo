import { DocsLayout } from "@/components/geistdocs/docs-layout";
import { openapiPages } from "@/lib/geistdocs/source";

const Layout = ({ children }: { children: React.ReactNode }) => {
  return <DocsLayout tree={openapiPages.pageTree}>{children}</DocsLayout>;
};

export default Layout;
