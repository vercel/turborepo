import { HomeLayout } from "@/components/geistdocs/home-layout";
import { getRootLang } from "@/lib/geistdocs/root-params";
import { source } from "@/lib/geistdocs/source";

const Layout = async ({ children }: LayoutProps<"/[lang]">) => {
  const lang = await getRootLang();

  return (
    <HomeLayout tree={source.pageTree[lang]}>
      <div className="bg-background-200 pt-0 pb-32">{children}</div>
    </HomeLayout>
  );
};

export default Layout;
