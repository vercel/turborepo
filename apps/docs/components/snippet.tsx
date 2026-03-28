"use client";

import { useState } from "react";
import { Check, Copy } from "lucide-react";
import { cn } from "@/lib/utils";

interface SnippetProps {
  code: string;
  className?: string;
  width?: string | number;
}

export function Snippet({ code, className }: SnippetProps) {
  const [copied, setCopied] = useState(false);

  const copyToClipboard = async () => {
    try {
      await navigator.clipboard.writeText(code);
      setCopied(true);
      setTimeout(() => {
        setCopied(false);
      }, 2000);
    } catch (err) {
      // eslint-disable-next-line no-console -- Purposeful.
      console.error("Failed to copy text: ", err);
    }
  };

  return (
    <div
      className={cn(
        "snippet relative rounded-md bg-card text-foreground overflow-hidden",
        className
      )}
    >
      <pre className="!bg-transparent p-2.5 sm:pl-4 pr-12 text-sm font-mono">
        $ {code}
      </pre>
      <button
        type="button"
        aria-label="Copy to clipboard"
        onClick={() => void copyToClipboard()}
        className="absolute right-2 top-2 p-1 sm:p-2 rounded-md hover:bg-accent transition-colors"
      >
        <span className="flex items-center justify-center">
          {copied ? (
            <Check size={16} className="text-green-600" />
          ) : (
            <Copy size={16} className="text-muted-foreground" />
          )}
        </span>
      </button>
    </div>
  );
}
