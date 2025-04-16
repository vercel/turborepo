"use client";

import { Button } from "./button";

export const CopyToMarkdown = ({
  markdownContent,
}: {
  markdownContent: string;
}) => {
  return (
    <Button
      variant="outline"
      onClick={() => navigator.clipboard.writeText(markdownContent)}
    >
      Copy .mdx
    </Button>
  );
};
