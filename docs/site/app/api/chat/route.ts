import {
  convertToModelMessages,
  createUIMessageStream,
  createUIMessageStreamResponse,
  generateText,
  stepCountIs,
  streamText
} from "ai";
import { createRagTools } from "./tools";
import type { MyUIMessage } from "./types";
import { createSystemPrompt } from "./utils";

export const maxDuration = 800;

// Cheaper model for RAG retrieval, better model for generation
const RAG_MODEL = "openai/gpt-4.1-mini";
const GENERATION_MODEL = "anthropic/claude-sonnet-4-20250514";

type RequestBody = {
  messages: MyUIMessage[];
  currentRoute: string;
  pageContext?: {
    title: string;
    url: string;
    content: string;
  };
};

export async function POST(req: Request) {
  try {
    const { messages, currentRoute, pageContext }: RequestBody =
      await req.json();

    // Filter out UI-only page context messages (they're just visual feedback)
    const actualMessages = messages.filter(
      (msg) => !msg.metadata?.isPageContext
    );

    // If pageContext is provided, prepend it to the last user message
    let processedMessages = actualMessages;

    if (pageContext && actualMessages.length > 0) {
      const lastMessage = actualMessages.at(-1);

      if (!lastMessage) {
        return new Response(
          JSON.stringify({
            error: "No last message found"
          }),
          { status: 500 }
        );
      }

      if (lastMessage.role === "user") {
        // Extract text content from the message parts
        const userQuestion = lastMessage.parts
          .filter((part) => part.type === "text")
          .map((part) => part.text)
          .join("\n");

        processedMessages = [
          ...actualMessages.slice(0, -1),
          {
            ...lastMessage,
            parts: [
              {
                type: "text",
                text: `Here's the content from the current page:

**Page:** ${pageContext.title}
**URL:** ${pageContext.url}

---

${pageContext.content}

---

User question: ${userQuestion}`
              }
            ]
          }
        ];
      }
    }

    const stream = createUIMessageStream({
      originalMessages: messages,
      execute: async ({ writer }) => {
        // Extract user question for RAG query
        const userQuestion =
          processedMessages
            .at(-1)
            ?.parts.filter((p) => p.type === "text")
            .map((p) => p.text)
            .join(" ") || "";

        // Stage 1: Use cheaper model for RAG retrieval (no streaming)
        const ragResult = await generateText({
          model: RAG_MODEL,
          messages: [{ role: "user", content: userQuestion }],
          tools: createRagTools(),
          stopWhen: stepCountIs(2),
          toolChoice: { type: "tool", toolName: "search_docs" }
        });

        // Extract retrieved documentation from tool results
        const retrievedDocs = ragResult.steps
          .flatMap((step) => step.toolResults)
          .map((result) => {
            // Handle both static tool results (with output) and dynamic results
            if ("output" in result) {
              return result.output;
            }
            return null;
          })
          .filter(Boolean)
          .join("\n\n---\n\n");

        // Collect source URLs from RAG results
        const sourceUrls: Array<{ url: string; title: string }> = [];
        for (const step of ragResult.steps) {
          for (const toolResult of step.toolResults) {
            if (!("output" in toolResult)) continue;
            const output = toolResult.output;
            if (
              toolResult.toolName === "search_docs" &&
              typeof output === "string"
            ) {
              const urlMatches = output.match(/URL: ([^\n]+)/g);
              if (urlMatches) {
                urlMatches.forEach((match) => {
                  const url = match.replace("URL: ", "").trim();
                  const titleMatch = output
                    .split(match)[0]
                    .match(/\*\*([^*]+)\*\*\s*$/);
                  const title = titleMatch ? titleMatch[1] : url;
                  sourceUrls.push({ url, title });
                });
              }
            }
          }
        }

        // Stage 2: Use better model for generation with retrieved context
        const result = streamText({
          model: GENERATION_MODEL,
          messages: convertToModelMessages([
            ...processedMessages.slice(0, -1),
            {
              role: "user",
              parts: [
                {
                  type: "text",
                  text: `Retrieved documentation:\n\n${retrievedDocs}\n\n---\n\nUser question: ${userQuestion}`
                }
              ]
            }
          ]),
          system: createSystemPrompt(currentRoute)
        });

        // Merge the generation stream first (this creates the message)
        await writer.merge(result.toUIMessageStream());

        // Then append sources to the same message
        sourceUrls.forEach((source, index) => {
          writer.write({
            type: "source-url",
            sourceId: `doc-${index}-${source.url}`,
            url: source.url,
            title: source.title
          });
        });
      }
    });

    return createUIMessageStreamResponse({ stream });
  } catch (error) {
    console.error("AI chat API error:", error);

    return new Response(
      JSON.stringify({
        error: "Failed to process chat request. Please try again."
      }),
      {
        status: 500,
        headers: { "Content-Type": "application/json" }
      }
    );
  }
}
