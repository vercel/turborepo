import { createDocsMarkdownRoute } from "@vercel/geistdocs/routes/llms";
import { openapiGeistdocsSource } from "@/lib/geistdocs/source";
import { siteUrl } from "@/lib/geistdocs/site-url";

const route = createDocsMarkdownRoute({
  source: openapiGeistdocsSource
});

export const generateStaticParams = route.generateStaticParams;

export const GET: typeof route.GET = async (request, context) => {
  const response = await route.GET(request, context);
  const link = response.headers.get("link");

  if (link) {
    response.headers.set(
      "link",
      link.replace(new URL(request.url).origin, siteUrl.origin)
    );
  }

  return response;
};
