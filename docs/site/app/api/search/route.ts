import { createSearchAPI } from "fumadocs-core/search/server";
import { repoDocsPages } from "../../source";
import { openapiPages } from "../../(openapi)/docs/openapi/source";

export const { GET } = createSearchAPI("advanced", {
  language: "english",
  indexes: [
    ...repoDocsPages.getPages().map((page) => ({
      title: page.data.title,
      description: page.data.description,
      url: page.url,
      id: page.url,
      structuredData: page.data.structuredData
    })),
    ...openapiPages.getPages().map((page) => ({
      title: page.data.title,
      description: page.data.description,
      url: page.url,
      id: page.url,
      structuredData: page.data.structuredData
    }))
  ]
});
