import {
  LanguageModelV1,
  LanguageModelV1CallWarning,
  LanguageModelV1FinishReason,
  LanguageModelV1StreamPart
} from "@ai-sdk/provider";
import {
  FetchFunction,
  ParseResult,
  combineHeaders,
  createEventSourceResponseHandler,
  createJsonResponseHandler,
  postJsonToApi
} from "@ai-sdk/provider-utils";
import { z } from "zod";
import { CustomChatSettings } from "./custom-chat-settings";

type CustomChatConfig = {
  provider: string;
  baseURL: string;
  headers: () => Record<string, string | undefined>;
  fetch?: FetchFunction;
  generateId: () => string;
};

export class CustomChatLanguageModel implements LanguageModelV1 {
  readonly specificationVersion = "v1";
  readonly defaultObjectGenerationMode = "json";

  readonly modelId: string;
  readonly settings: CustomChatSettings;

  private readonly config: CustomChatConfig;

  constructor(
    modelId: string,
    settings: CustomChatSettings,
    config: CustomChatConfig
  ) {
    this.modelId = modelId;
    this.settings = settings;
    this.config = config;
  }

  get provider(): string {
    return this.config.provider;
  }

  private getArgs({
    mode,
    prompt,
    maxTokens,
    temperature,
    topP,
    frequencyPenalty,
    presencePenalty,
    responseFormat,
    seed
  }: Parameters<LanguageModelV1["doGenerate"]>[0]) {
    const type = mode.type;

    const warnings: LanguageModelV1CallWarning[] = [];

    if (
      frequencyPenalty != null &&
      (frequencyPenalty < -2 || frequencyPenalty > 2)
    ) {
      warnings.push({
        type: "other",
        message: "Frequency penalty must be between -2 and 2"
      });
    }

    if (
      presencePenalty != null &&
      (presencePenalty < -2 || presencePenalty > 2)
    ) {
      warnings.push({
        type: "other",
        message: "Presence penalty must be between -2 and 2"
      });
    }

    const baseArgs = {
      model: this.modelId,
      messages: prompt,
      max_tokens: maxTokens ?? this.settings.maxTokens,
      temperature: temperature ?? this.settings.temperature,
      top_p: topP ?? this.settings.topP,
      frequency_penalty: frequencyPenalty ?? this.settings.frequencyPenalty,
      presence_penalty: presencePenalty ?? this.settings.presencePenalty,
      response_format: responseFormat ?? this.settings.responseFormat,
      stop: this.settings.stop,
      user: this.settings.user,
      seed
    };

    switch (type) {
      case "regular": {
        return { args: baseArgs, warnings };
      }

      case "object-json": {
        return {
          args: {
            ...baseArgs,
            response_format: { type: "json_object" as const }
          },
          warnings
        };
      }

      case "object-tool": {
        return {
          args: {
            ...baseArgs,
            tools: [{ type: "function" as const, function: mode.tool }],
            tool_choice: {
              type: "function" as const,
              function: { name: mode.tool.name }
            }
          },
          warnings
        };
      }

      default: {
        const _exhaustiveCheck: never = type;
        throw new Error(`Unsupported type: ${_exhaustiveCheck}`);
      }
    }
  }

  async doGenerate(
    options: Parameters<LanguageModelV1["doGenerate"]>[0]
  ): Promise<Awaited<ReturnValue<LanguageModelV1["doGenerate"]>>> {
    const { args, warnings } = this.getArgs(options);

    const { responseHeaders, value: response } = await postJsonToApi({
      url: `${this.config.baseURL}/chat/completions`,
      headers: combineHeaders(this.config.headers(), options.headers),
      body: args,
      failedResponseHandler: createJsonResponseHandler({
        errorSchema: z.object({
          error: z.object({
            message: z.string(),
            type: z.string().optional(),
            code: z.string().optional()
          })
        }),
        errorToMessage: (data) => data.error.message
      }),
      successfulResponseHandler: createJsonResponseHandler({
        schema: z.object({
          choices: z.array(
            z.object({
              message: z.object({
                role: z.literal("assistant"),
                content: z.string().nullable(),
                tool_calls: z
                  .array(
                    z.object({
                      id: z.string(),
                      type: z.literal("function"),
                      function: z.object({
                        name: z.string(),
                        arguments: z.string()
                      })
                    })
                  )
                  .optional()
              }),
              index: z.number(),
              finish_reason: z.string().optional()
            })
          ),
          usage: z.object({
            prompt_tokens: z.number(),
            completion_tokens: z.number(),
            total_tokens: z.number()
          })
        })
      }),
      abortSignal: options.abortSignal,
      fetch: this.config.fetch
    });

    const { choices, usage } = response;
    const choice = choices[0];

    return {
      text: choice.message.content ?? "",
      toolCalls: choice.message.tool_calls?.map((toolCall) => ({
        toolCallType: "function" as const,
        toolCallId: toolCall.id,
        toolName: toolCall.function.name,
        args: toolCall.function.arguments
      })),
      finishReason: mapFinishReason(choice.finish_reason),
      usage: {
        promptTokens: usage.prompt_tokens,
        completionTokens: usage.completion_tokens
      },
      rawCall: { rawPrompt: args, rawSettings: {} },
      warnings,
      response: {
        headers: responseHeaders
      }
    };
  }

  async doStream(
    options: Parameters<LanguageModelV1["doStream"]>[0]
  ): Promise<Awaited<ReturnValue<LanguageModelV1["doStream"]>>> {
    const { args, warnings } = this.getArgs(options);

    const { responseHeaders, value: response } = await postJsonToApi({
      url: `${this.config.baseURL}/chat/completions`,
      headers: combineHeaders(this.config.headers(), options.headers),
      body: {
        ...args,
        stream: true
      },
      failedResponseHandler: createJsonResponseHandler({
        errorSchema: z.object({
          error: z.object({
            message: z.string(),
            type: z.string().optional(),
            code: z.string().optional()
          })
        }),
        errorToMessage: (data) => data.error.message
      }),
      successfulResponseHandler: createEventSourceResponseHandler({
        schema: z.object({
          choices: z.array(
            z.object({
              delta: z.object({
                role: z.enum(["assistant"]).optional(),
                content: z.string().nullable().optional(),
                tool_calls: z
                  .array(
                    z.object({
                      index: z.number(),
                      id: z.string().optional(),
                      type: z.literal("function").optional(),
                      function: z
                        .object({
                          name: z.string().optional(),
                          arguments: z.string().optional()
                        })
                        .optional()
                    })
                  )
                  .optional()
              }),
              finish_reason: z.string().nullable().optional(),
              index: z.number()
            })
          ),
          usage: z
            .object({
              prompt_tokens: z.number(),
              completion_tokens: z.number(),
              total_tokens: z.number()
            })
            .optional()
        })
      }),
      abortSignal: options.abortSignal,
      fetch: this.config.fetch
    });

    let finishReason: LanguageModelV1FinishReason = "other";
    let usage: { promptTokens: number; completionTokens: number } = {
      promptTokens: Number.NaN,
      completionTokens: Number.NaN
    };

    const toolCalls: Array<{
      id: string;
      type: "function";
      function: { name: string; arguments: string };
    }> = [];

    return {
      stream: response.pipeThrough(
        new TransformStream<
          ParseResult<z.infer<typeof response.schema>>,
          LanguageModelV1StreamPart
        >({
          transform(chunk, controller) {
            if (!chunk.success) {
              controller.enqueue({ type: "error", error: chunk.error });
              return;
            }

            const value = chunk.value;

            if (value.usage) {
              usage = {
                promptTokens: value.usage.prompt_tokens,
                completionTokens: value.usage.completion_tokens
              };
            }

            const choice = value.choices[0];

            if (choice?.finish_reason) {
              finishReason = mapFinishReason(choice.finish_reason);
            }

            if (choice?.delta?.content) {
              controller.enqueue({
                type: "text-delta",
                textDelta: choice.delta.content
              });
            }

            if (choice?.delta?.tool_calls) {
              for (const toolCallDelta of choice.delta.tool_calls) {
                if (toolCallDelta.index >= toolCalls.length) {
                  toolCalls.push({
                    id: toolCallDelta.id!,
                    type: "function",
                    function: { name: "", arguments: "" }
                  });
                }

                const toolCall = toolCalls[toolCallDelta.index];

                if (toolCallDelta.function?.name) {
                  toolCall.function.name = toolCallDelta.function.name;
                }

                if (toolCallDelta.function?.arguments) {
                  toolCall.function.arguments +=
                    toolCallDelta.function.arguments;
                }

                controller.enqueue({
                  type: "tool-call-delta",
                  toolCallType: "function",
                  toolCallId: toolCall.id,
                  toolName: toolCall.function.name,
                  argsTextDelta: toolCallDelta.function?.arguments ?? ""
                });
              }
            }
          },

          flush(controller) {
            controller.enqueue({
              type: "finish",
              finishReason,
              usage,
              response: {
                headers: responseHeaders
              }
            });
          }
        })
      ),
      rawCall: { rawPrompt: args, rawSettings: {} },
      warnings
    };
  }
}

function mapFinishReason(
  finishReason: string | null | undefined
): LanguageModelV1FinishReason {
  switch (finishReason) {
    case "stop":
      return "stop";
    case "length":
      return "length";
    case "function_call":
    case "tool_calls":
      return "tool-calls";
    case "content_filter":
      return "content-filter";
    default:
      return "other";
  }
}
