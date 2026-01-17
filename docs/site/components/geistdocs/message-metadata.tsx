import { isToolUIPart } from "ai";
import type { MyUIMessage } from "@/app/api/chat/types";
import { Shimmer } from "../ai-elements/shimmer";
import {
  Source,
  Sources,
  SourcesContent,
  SourcesTrigger
} from "../ai-elements/sources";
import { Spinner } from "../ui/spinner";

type MessageMetadataProps = {
  messageId?: string;
  parts: MyUIMessage["parts"];
  inProgress: boolean;
  isStreaming?: boolean;
};

export const MessageMetadata = ({
  parts,
  inProgress,
  isStreaming
}: MessageMetadataProps) => {
  // Pull out last part that is either text or tool call
  const lastPart = parts
    .filter((part) => part.type === "text" || isToolUIPart(part))
    .at(-1);

  const reasoning = parts.at(-1)?.type === "reasoning";

  const sources = Array.from(
    new Map(
      parts
        .filter((part) => part.type === "source-url")
        .map((part) => [part.url, part])
    ).values()
  );

  const tool = lastPart && isToolUIPart(lastPart) ? lastPart : null;

  // Show spinner only when waiting for content (no text/tool yet and not streaming)
  if (!lastPart && sources.length === 0 && !isStreaming) {
    return (
      <div className="flex items-center gap-2">
        <Spinner />{" "}
        {reasoning ? <Shimmer className="text-xs">Thinking...</Shimmer> : ""}
      </div>
    );
  }

  // Only show sources if there's also text content (avoids duplicate from sources-only messages)
  if (sources.length > 0 && lastPart && !(tool && inProgress)) {
    return (
      <Sources>
        <SourcesTrigger count={sources.length} />
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

  if (tool && inProgress) {
    // Show user-friendly message for search_docs tool
    const isSearching = tool.type === "tool-search_docs";
    return (
      <div className="flex items-center gap-2">
        <Spinner />
        <Shimmer>
          {isSearching
            ? "Searching sources..."
            : tool.type.replace("tool-", "")}
        </Shimmer>
      </div>
    );
  }

  if (!tool && sources.length === 0) {
    return null;
  }

  return null;
};
