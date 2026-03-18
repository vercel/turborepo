import "../global.css";
import type { Metadata } from "next";
import { VercelToolbar } from "@vercel/toolbar/next";
import { FaviconHandler } from "@/components/favicon-handler";
import { Footer } from "@/components/geistdocs/footer";
import { Navbar } from "@/components/geistdocs/navbar";
import { GeistdocsProvider } from "@/components/geistdocs/provider";
import { basePath } from "@/geistdocs";
import { mono, sans } from "@/lib/geistdocs/fonts";
import { cn } from "@/lib/utils";

export const metadata: Metadata = {
  icons: {
    icon: "/images/product-icons/repo-dark-32x32.png"
  }
};

const Layout = async ({ children, params }: LayoutProps<"/[lang]">) => {
  const { lang } = await params;
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
        <GeistdocsProvider basePath={basePath} lang={lang}>
          <a
            className="fixed left-4 top-4 z-50 -translate-y-full rounded-md bg-background px-4 py-2 text-sm font-medium text-foreground shadow-md transition focus:translate-y-0"
            href="#main-content"
          >
            Skip to content
          </a>
          <Navbar />
          <div id="main-content">{children}</div>
          <Footer />
        </GeistdocsProvider>
        {shouldInjectToolbar ? <VercelToolbar /> : null}
      </body>
    </html>
  );
};

export default Layout;
