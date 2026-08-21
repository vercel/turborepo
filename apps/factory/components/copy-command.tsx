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
    <div className="copyCommand">
      <code aria-label={label ?? "Terminal command"}>{command}</code>
      <Button
        aria-label={copied ? "Copied" : `Copy ${label ?? "command"}`}
        className="copyCommandButton"
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
