"use client";

import algo from "algoliasearch/lite";
import type { SharedProps } from "fumadocs-ui/components/dialog/search";
import FumaSearchDialog from "fumadocs-ui/components/dialog/search-algolia";
import { gitHubRepoUrl } from "@/lib/constants";

// Make sure these environment variables are defined in your environment
const appId = process.env.NEXT_PUBLIC_ALGOLIA_APP_ID || "";
const readKey = process.env.NEXT_PUBLIC_ALGOLIA_READ_KEY || "";
const indexName = process.env.NEXT_PUBLIC_ALGOLIA_INDEX || "";

const client = algo(appId, readKey);
const index = client.initIndex(indexName);

export function SearchDialog(props: SharedProps): JSX.Element {
  return (
    <FumaSearchDialog
      index={index}
      {...props}
      links={[
        ["Home", "/"],
        ["Turborepo documentation", "/docs"],
        ["Blog", "/blog"],
        ["Changelog", `${gitHubRepoUrl}/releases`],
        ["Github", gitHubRepoUrl],
        ["Community", "https://community.vercel.com/tag/turborepo"],
      ]}
    />
  );
}
