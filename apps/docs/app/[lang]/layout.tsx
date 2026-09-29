import "../global.css";
import type { Metadata } from "next";
import { VercelToolbar } from "@vercel/toolbar/next";
import { Footer } from "@vercel/geistdocs/footer";
import { Navbar } from "@vercel/geistdocs/navbar";
import { FaviconHandler } from "@/components/favicon-handler";
import { GeistdocsProvider } from "@/components/geistdocs/provider";
import { translations } from "@/geistdocs";
import { config } from "@/lib/geistdocs/config";
import { mono, sans } from "@/lib/geistdocs/fonts";
import { getRootLang } from "@/lib/geistdocs/root-params";
import { siteUrl } from "@/lib/geistdocs/site-url";
import { cn } from "@/lib/utils";

export const generateStaticParams = () =>
  Object.keys(translations).map((lang) => ({ lang }));

export const metadata: Metadata = {
  metadataBase: siteUrl,
  icons: {
    icon: "/images/product-icons/repo-dark-32x32.png"
  }
};

const Layout = async ({ children }: LayoutProps<"/[lang]">) => {
  const lang = await getRootLang();
  const shouldInjectToolbar = process.env.NODE_ENV === "development";

  return (
    <html
      className={cn(sans.variable, mono.variable, "antialiased")}
      lang={lang}
      suppressHydrationWarning
    >
      <head>
        <FaviconHandler />
      </head>
      <body>
        <GeistdocsProvider basePath={config.basePath} lang={lang}>
          <a
            className="sr-only focus:not-sr-only focus:fixed focus:left-4 focus:top-4 focus:z-50 focus:rounded-md focus:bg-background focus:px-4 focus:py-2 focus:text-sm focus:font-medium focus:text-foreground focus:shadow-md"
            href="#main-content"
          >
            Skip to content
          </a>
          <Navbar config={config} />
          <div id="main-content">{children}</div>
          <Footer />
        </GeistdocsProvider>
        {shouldInjectToolbar ? <VercelToolbar /> : null}
      </body>
    </html>
  );
};

export default Layout;
