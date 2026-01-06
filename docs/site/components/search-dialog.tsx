"use client";

import type { SharedProps } from "fumadocs-ui/components/dialog/search";
import DefaultSearchDialog from "fumadocs-ui/components/dialog/search-default";
import { gitHubRepoUrl } from "#lib/constants.ts";

export function SearchDialog(props: SharedProps): JSX.Element {
  return (
    <DefaultSearchDialog
      type="fetch"
      api="/api/search"
      {...props}
      links={[
        ["Home", "/"],
        ["Turborepo documentation", "/docs"],
        ["Blog", "/blog"],
        ["Changelog", `${gitHubRepoUrl}/releases`],
        ["Github", gitHubRepoUrl],
        ["Community", "https://community.vercel.com/tag/turborepo"]
      ]}
    />
  );
}
