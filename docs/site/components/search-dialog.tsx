"use client";

import { gitHubRepoUrl } from "@/lib/constants";
import algo from "algoliasearch/lite";
import type { SharedProps } from "fumadocs-ui/components/dialog/search";
import FumaSearchDialog from "fumadocs-ui/components/dialog/search-algolia";

const client = algo(
  process.env.NEXT_PUBLIC_ALGOLIA_APP_ID,
  process.env.NEXT_PUBLIC_ALGOLIA_READ_KEY!
);

const index = client.initIndex(process.env.NEXT_PUBLIC_ALGOLIA_INDEX!);

export function SearchDialog(props: SharedProps): JSX.Element {
  return (
    <FumaSearchDialog
      index={index}
      {...props}
      links={[
        ["Home", "/"],
        ["Turborepo documentation", "/repo/docs"],
        ["Blog", "/blog"],
        ["Changelog", `${gitHubRepoUrl}/releases`],
        ["Github", gitHubRepoUrl],
        ["Vercel Community", "https://vercel.community/tag/turborepo"],
      ]}
    />
  );
}
