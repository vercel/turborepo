import { createAgentsRoute } from "@vercel/geistdocs/routes/agents";
import { config } from "@/lib/geistdocs/config";
import { siteUrl } from "@/lib/geistdocs/site-url";

const route = createAgentsRoute({
  config,
  transform: (markdown, { request }) =>
    markdown
      .replaceAll(request.nextUrl.origin, siteUrl.origin)
      .replace(
        /- \[Full documentation context\]\(([^)]+\/llms)\.txt\): All configured documentation as Markdown/,
        "- [Documentation index]($1.txt): Links to individual documentation pages as Markdown\n- [Full documentation context]($1-full.txt): Complete documentation corpus as Markdown"
      )
});

export const { GET, generateStaticParams } = route;
