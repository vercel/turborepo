import { isToolUIPart } from "ai";
import type { MyUIMessage } from "@/app/api/chat/types";
import {
  Source,
  Sources,
  SourcesContent,
  SourcesTrigger
} from "../ai-elements/sources";

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

  const sources = Array.from(
    new Map(
      parts
        .filter((part) => part.type === "source-url")
        .map((part) => [part.url, part])
    ).values()
  );

  const tool = lastPart && isToolUIPart(lastPart) ? lastPart : null;

  // Show sources once they exist
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

  return null;
};
