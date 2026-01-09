import "../global.css";
import type { Metadata } from "next";
import { VercelToolbar } from "@vercel/toolbar/next";
import { Footer } from "@/components/geistdocs/footer";
import { Navbar } from "@/components/geistdocs/navbar";
import { GeistdocsProvider } from "@/components/geistdocs/provider";
import { basePath } from "@/geistdocs";
import { mono, sans } from "@/lib/geistdocs/fonts";
import { cn } from "@/lib/utils";

export const metadata: Metadata = {
  icons: {
    icon: [
      {
        url: "/images/product-icons/repo-light-32x32.png",
        media: "(prefers-color-scheme: light)"
      },
      {
        url: "/images/product-icons/repo-dark-32x32.png",
        media: "(prefers-color-scheme: dark)"
      }
    ]
  }
};

const Layout = async ({ children, params }: LayoutProps<"/[lang]">) => {
  const { lang } = await params;
  const shouldInjectToolbar = process.env.NODE_ENV === "development";

  return (
    <html
      className={cn(sans.variable, mono.variable, "scroll-smooth antialiased")}
      lang={lang}
      suppressHydrationWarning
    >
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
