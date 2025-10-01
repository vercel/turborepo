"use client";

import { useState } from "react";
import { Copy, Check } from "lucide-react";
import { Button } from "./button";

export const CopyToMarkdown = ({
  markdownContent,
}: {
  markdownContent: string;
}) => {
  const [copied, setCopied] = useState(false);

  const handleCopy = async () => {
    await navigator.clipboard.writeText(markdownContent);
    setCopied(true);

    // Reset back to copy icon after 2 seconds
    setTimeout(() => {
      setCopied(false);
    }, 2000);
  };

  return (
    <Button
      variant="ghost"
      size="sm"
      className={"text-xs hidden sm:flex"}
      onClick={() => {
        void handleCopy();
      }}
    >
      {copied ? (
        <Check className="w-4 h-4 mr-1" />
      ) : (
        <Copy className="w-4 h-4 mr-1" />
      )}
      Copy page
    </Button>
  );
};
