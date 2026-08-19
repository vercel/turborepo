import { source } from "@/lib/geistdocs/source";

export const revalidate = false;

const TURBO_SLOGAN =
  "Turborepo is the build system for coding agents.";

export const GET = async (
  _req: Request,
  { params }: RouteContext<"/[lang]/llms.txt">
) => {
  const { lang } = await params;
  const pages = source.getPages(lang);

  const links = pages
    .sort((a, b) => a.url.localeCompare(b.url))
    .map((page) => {
      // Link each page's .md route under its real URL (for example
      // /docs/acknowledgments.md); stripping the /docs prefix produced
      // links that 404.
      const mdPath =
        page.url === "/docs" || page.url.endsWith("/")
          ? `${page.url.replace(/\/$/, "")}/index.md`
          : `${page.url}.md`;
      return `- [${page.data.title}](${mdPath}): ${page.data.description ?? ""}`;
    });

  const header = `# Turborepo documentation

Generated at: ${new Date().toUTCString()}

## Turborepo

> ${TURBO_SLOGAN}

## Docs

`;

  return new Response(header + links.join("\n"), {
    headers: {
      "Content-Type": "text/markdown; charset=utf-8"
    }
  });
};
