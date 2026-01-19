import type { NextRequest } from "next/server";
import { source } from "@/lib/geistdocs/source";
import { trackMdRequest } from "@/lib/md-tracking";

export const revalidate = false;

const TURBO_SLOGAN =
  "Turborepo is a build system optimized for JavaScript and TypeScript, written in Rust.";

export const GET = async (
  req: NextRequest,
  { params }: RouteContext<"/[lang]/llms.txt">
) => {
  const { lang } = await params;
  const pages = source.getPages(lang);

  // Track markdown request (fire-and-forget)
  const userAgent = req.headers.get("user-agent");
  const referer = req.headers.get("referer");
  const acceptHeader = req.headers.get("accept");
  void trackMdRequest({
    path: "/llms.txt",
    userAgent,
    referer,
    acceptHeader
  });

  const links = pages
    .sort((a, b) => a.url.localeCompare(b.url))
    .map((page) => {
      let mdPath = page.url.replace(/^\/docs/, "");
      // Handle index pages
      if (mdPath === "" || mdPath.endsWith("/")) {
        mdPath = mdPath + "index.md";
      } else {
        mdPath = mdPath + ".md";
      }
      return `- [${page.data.title}](${mdPath}): ${page.data.description ?? ""}`;
    });

  const header = `# Turborepo documentation

Generated at: ${new Date().toUTCString()}

## Turborepo

> ${TURBO_SLOGAN}

## Docs

`;

  return new Response(header + links.join("\n"));
};
