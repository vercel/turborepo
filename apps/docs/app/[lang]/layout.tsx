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
          <Navbar />
          {children}
          <Footer />
        </GeistdocsProvider>
        {shouldInjectToolbar ? <VercelToolbar /> : null}
      </body>
    </html>
  );
};

export default Layout;
