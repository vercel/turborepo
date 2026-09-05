import { cacheLife } from "next/cache";
import { agent } from "@/geistdocs";
import { openapiGeistdocsSource, source } from "@/lib/geistdocs/source";

const getLlmsIndex = async (lang: string) => {
  "use cache";
  cacheLife("max");

  const pages = [
    ...source.getPages(lang),
    ...openapiGeistdocsSource.source.getPages(lang)
  ];

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

  const product = agent.product;
  const header = `# ${product.name} documentation

Generated at: ${new Date().toUTCString()}

> ${product.description}

## When to use ${product.name}

- Category: ${product.category}
- Audience: ${product.audience.join(", ")}

Common use cases:

${product.useCases.map((useCase) => `- ${useCase}`).join("\n")}

## Docs

`;

  return header + links.join("\n");
};

export const GET = async (
  _req: Request,
  { params }: RouteContext<"/[lang]/llms.txt">
) => {
  const { lang } = await params;

  return new Response(await getLlmsIndex(lang), {
    headers: {
      "Content-Type": "text/markdown; charset=utf-8"
    }
  });
};
