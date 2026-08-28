import { createProxy } from "@vercel/geistdocs/proxy";
import { config as geistdocsConfig } from "@/lib/geistdocs/config";
import { trackMdRequest } from "@/lib/md-tracking";

const proxy = createProxy({
  config: geistdocsConfig,
  markdownRoutes: [
    {
      from: "/docs/openapi/*path",
      to: "/[lang]/openapi.mdx/*path"
    },
    { from: "/docs/*path", to: "/[lang]/llms.mdx/*path" }
  ],
  trackMarkdownRequest: trackMdRequest
});

export const config = {
  matcher: [
    "/((?!api(?:/|$)|_next/static|_next/image|favicon.ico|feed.xml|sitemap.xml|robots.txt|images(?:/|$)|og-image\\.png|schema\\.json|schema\\.v\\d+\\.json|microfrontends/schema\\.json|\\.well-known/security\\.txt).*)"
  ]
};

export default proxy;
