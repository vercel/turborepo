import { source } from "@/lib/geistdocs/source";

export const revalidate = false;

const TURBO_SLOGAN =
  "Turborepo is a build system optimized for JavaScript and TypeScript, written in Rust.";

export const GET = async (
  _req: Request,
  { params }: RouteContext<"/[lang]/llms.txt">
) => {
  const { lang } = await params;
  const pages = source.getPages(lang);


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
