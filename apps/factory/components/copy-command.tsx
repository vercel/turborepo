"use client";

import { useCallback, useState } from "react";

import { Button } from "./ui/button";

interface CopyCommandProps {
  readonly command: string;
  readonly label?: string;
}

export function CopyCommand({ command, label }: CopyCommandProps) {
  const [copied, setCopied] = useState(false);

  const copy = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(command);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      // Ignore clipboard failures; the user can still select the text manually.
    }
  }, [command]);

  return (
    <div className="mt-3 flex items-center justify-between gap-2 rounded-md border border-border bg-background px-3 py-2">
      <code
        className="overflow-hidden text-ellipsis whitespace-nowrap font-mono text-xs text-foreground"
        aria-label={label ?? "Terminal command"}
      >
        {command}
      </code>
      <Button
        aria-label={copied ? "Copied" : `Copy ${label ?? "command"}`}
        className="h-auto shrink-0 px-2 py-0.5 text-xs"
        onClick={() => void copy()}
        size="sm"
        type="button"
        variant="ghost"
      >
        {copied ? "Copied" : "Copy"}
      </Button>
    </div>
  );
}
