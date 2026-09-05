import { DocsLayout } from "@/components/geistdocs/docs-layout";
import { getRootLang } from "@/lib/geistdocs/root-params";
import { source } from "@/lib/geistdocs/source";

const Layout = async ({ children }: LayoutProps<"/[lang]/docs">) => {
  const lang = await getRootLang();

  return <DocsLayout tree={source.pageTree[lang]}>{children}</DocsLayout>;
};

export default Layout;
