import { createSearchRoute } from "@vercel/geistdocs/routes/search";
import { config } from "@/lib/geistdocs/config";
import {
  geistdocsSource,
  openapiGeistdocsSource
} from "@/lib/geistdocs/source";

export const GET = createSearchRoute({
  config,
  sources: [geistdocsSource, openapiGeistdocsSource]
});
