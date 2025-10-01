"use client";

import { useState } from "react";
import { Check, Copy } from "lucide-react";
import { cn } from "./cn";

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
        "snippet relative rounded-md bg-[var(--ds-background-100)] text-black dark:text-white overflow-hidden",
        className
      )}
    >
      <pre className="geist-overflow-scroll-y !bg-[var(--ds-background-100)] p-2.5 sm:pl-4 pr-12 text-sm font-mono">
        $ {code}
      </pre>
      <button
        type="button"
        aria-label="Copy to clipboard"
        onClick={() => void copyToClipboard()}
        className="absolute right-2 top-2 p-1 sm:p-2 rounded-md hover:bg-gray-800 transition-colors"
      >
        <span className="flex items-center justify-center">
          {copied ? (
            <Check size={16} className="text-green-800" />
          ) : (
            <Copy size={16} />
          )}
        </span>
      </button>
    </div>
  );
}
