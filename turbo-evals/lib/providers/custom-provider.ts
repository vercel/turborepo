import { createOpenAICompatible } from "@ai-sdk/openai-compatible";
import { LanguageModel } from "ai";

export interface CustomProviderSettings {
  baseURL?: string;
  apiKey?: string;
  headers?: Record<string, string>;
}

export function createCustomProvider(options: CustomProviderSettings = {}) {
  const openaiCompatible = createOpenAICompatible({
    name: "custom-provider",
    baseURL: options.baseURL ?? "https://api.custom.ai/v1",
    apiKey: options.apiKey ?? process.env.CUSTOM_PROVIDER_API_KEY,
    headers: options.headers
  });

  return function (modelId: string): LanguageModel {
    try {
      const model = openaiCompatible(modelId);

      // Need to Create a wrapper for ai sdk v5
      const wrappedModel = {
        ...model,
        doGenerate: async (options: any) => {
          // define mode for ai sdk v5
          const mode = options.mode || { type: "regular" };
          const result = await model.doGenerate({ ...options, mode });
          if (result && typeof result === "object") {
            return {
              ...result,
              // Convert text to content array format for ai sdk v5
              content: result.text ? [{ type: "text", text: result.text }] : []
            };
          }
          return result;
        },
        doStream: async (options: any) => {
          // define mode for ai sdk v5
          const mode = options.mode || { type: "regular" };
          return model.doStream({ ...options, mode });
        }
      };

      return wrappedModel as unknown as LanguageModel;
    } catch (error) {
      console.error(
        `Error creating custom provider for model ${modelId}:`,
        error
      );
      throw error;
    }
  };
}

export const defaultCustomProvider = createCustomProvider();
