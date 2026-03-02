import { isToolUIPart } from "ai";
import { BookmarkIcon } from "lucide-react";
import type { MyUIMessage } from "@/app/api/chat/types";
import { Shimmer } from "../ai-elements/shimmer";
import {
  Source,
  Sources,
  SourcesContent,
  SourcesTrigger,
} from "../ai-elements/sources";

interface MessageMetadataProps {
  inProgress: boolean;
  parts: MyUIMessage["parts"];
}

export const MessageMetadata = ({
  parts,
  inProgress,
}: MessageMetadataProps) => {
  // Pull out last part that is either text or tool call
  const lastPart = parts
    .filter((part) => part.type === "text" || isToolUIPart(part))
    .at(-1);

  if (!lastPart) {
    return <Shimmer className="text-xs">Thinking...</Shimmer>;
  }

  const tool = isToolUIPart(lastPart) ? lastPart : null;
  const hasTextPart = parts.some((part) => part.type === "text");

  const sources = Array.from(
    new Map(
      parts
        .filter((part) => part.type === "source-url")
        .map((part) => [part.url, part])
    ).values()
  );

  // Show loading state when sources exist but text hasn't arrived yet
  if (sources.length > 0 && !hasTextPart && inProgress) {
    return <Shimmer className="text-xs">Searching sources...</Shimmer>;
  }

  if (sources.length > 0 && !(tool && inProgress)) {
    return (
      <Sources>
        <SourcesTrigger count={sources.length}>
          <BookmarkIcon className="size-4" />
          <p>Used {sources.length} sources</p>
        </SourcesTrigger>
        <SourcesContent>
          <ul className="flex flex-col gap-2">
            {sources.map((source) => (
              <li className="ml-4.5 list-disc pl-1" key={source.url}>
                <Source href={source.url} title={source.url}>
                  {source.title}
                </Source>
              </li>
            ))}
          </ul>
        </SourcesContent>
      </Sources>
    );
  }

  if (!tool && sources.length === 0) {
    return null;
  }

  return <div className="h-12" />;
};
