import { createProxy } from "@vercel/geistdocs/proxy";
import { config as geistdocsConfig } from "@/lib/geistdocs/config";
import { trackMdRequest } from "@/lib/md-tracking";

const proxy = createProxy({
  config: geistdocsConfig,
  trackMarkdownRequest: trackMdRequest
});

export const config = {
  matcher: [
    "/((?!api(?:/|$)|_next/static|_next/image|favicon.ico|feed.xml|sitemap.xml|robots.txt|schema\\.json|schema\\.v\\d+\\.json|microfrontends/schema\\.json).*)"
  ]
};

export default proxy;
