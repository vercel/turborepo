import { isToolUIPart } from "ai";
import { BookmarkIcon } from "lucide-react";
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

  if (!lastPart) {
    return (
      <div className="flex items-center gap-2">
        <Spinner />{" "}
        {reasoning ? <Shimmer className="text-xs">Thinking...</Shimmer> : ""}
      </div>
    );
  }

  const tool = isToolUIPart(lastPart) ? lastPart : null;

  const sources = Array.from(
    new Map(
      parts
        .filter((part) => part.type === "source-url")
        .map((part) => [part.url, part])
    ).values()
  );

  // Check if there's any text content in the message
  const hasTextContent = parts.some(
    (part) => part.type === "text" && part.text.length > 0
  );

  // Show loading state when sources exist but text hasn't started streaming yet
  if (sources.length > 0 && !hasTextContent && isStreaming) {
    return (
      <div className="flex flex-col gap-2">
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
        <div className="flex items-center gap-2">
          <Spinner />
          <Shimmer>Generating response...</Shimmer>
        </div>
      </div>
    );
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

  return <div className="h-12" />;
};
