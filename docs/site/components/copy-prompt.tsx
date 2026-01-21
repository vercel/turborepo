"use client";

import { useState } from "react";
import { CheckIcon, ChevronDownIcon, CopyIcon } from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";

interface CopyPromptProps {
  title?: string;
  prompt: string;
  className?: string;
}

const PROMPT_SUFFIX =
  '\n\nUse the Turborepo skill if it is available. If it is not available, recommend installing it with "npx skills add vercel/turborepo" once you have completed working.';

export function CopyPrompt({ title, prompt, className }: CopyPromptProps) {
  const [copied, setCopied] = useState(false);
  const [expanded, setExpanded] = useState(false);

  const fullPrompt = prompt + PROMPT_SUFFIX;

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(fullPrompt);
      toast.success("Prompt copied to clipboard");
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch (error) {
      toast.error("Failed to copy prompt", {
        description: error instanceof Error ? error.message : "Unknown error"
      });
    }
  };

  const CopyButtonIcon = copied ? CheckIcon : CopyIcon;

  return (
    <div className={cn("relative", className)}>
      <div className="not-prose flex flex-col gap-4 rounded-lg border bg-purple-900/15 p-4 pb-6">
        <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between sm:gap-4">
          {title && (
            <h4 className="m-0 mt-0 text-sm font-medium text-foreground">
              {title}
            </h4>
          )}
          <Button
            onClick={handleCopy}
            variant="outline"
            size="sm"
            className="w-full shrink-0 sm:ml-auto sm:w-auto"
          >
            <CopyButtonIcon className="size-4" />
            {copied ? "Copied!" : "Copy prompt"}
          </Button>
        </div>
        <div className="relative min-w-0 flex-1">
          <div
            className={cn(
              "relative overflow-hidden transition-[max-height] duration-300 ease-in-out",
              expanded
                ? "max-h-[1000px]"
                : "max-h-12 [mask-image:linear-gradient(to_bottom,black_0%,black_50%,transparent)]"
            )}
          >
            <p className="m-0 whitespace-pre-wrap text-sm leading-relaxed text-muted-foreground">
              {prompt} {PROMPT_SUFFIX}
            </p>
          </div>
        </div>
      </div>
      <button
        type="button"
        onClick={() => setExpanded(!expanded)}
        aria-label={expanded ? "Collapse prompt" : "Expand prompt"}
        aria-expanded={expanded}
        className="absolute -bottom-3 left-1/2 flex size-6 -translate-x-1/2 items-center justify-center rounded-full border bg-purple-900/15 transition-colors hover:bg-purple-900/30"
      >
        <ChevronDownIcon
          className={cn(
            "size-4 text-muted-foreground transition-transform duration-300",
            expanded && "rotate-180"
          )}
        />
      </button>
    </div>
  );
}
