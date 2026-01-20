import { LanguageModel } from "ai";
import { createCustomProvider } from "./providers/custom-provider";
import * as fs from "fs";
import * as path from "path";
export interface Model {
  name: string;
  model: LanguageModel;
}

export interface CustomProviderConfig {
  name: string;
  modelId: string;
  baseURL: string;
  apiKey?: string;
  headers?: Record<string, string>;
}

// Function to load custom providers from JSON file
function loadCustomProviders(): Model[] {
  const customModels: Model[] = [];

  try {
    const configPath = path.join(
      __dirname,
      "providers",
      "custom-providers.json"
    );

    // Check if the config file exists
    if (!fs.existsSync(configPath)) {
      console.log(
        "No custom providers configuration file found at:",
        configPath
      );
      return customModels;
    }

    const configData = fs.readFileSync(configPath, "utf8");
    const customProviderConfigs: CustomProviderConfig[] =
      JSON.parse(configData);

    for (const config of customProviderConfigs) {
      try {
        const provider = createCustomProvider({
          baseURL: config.baseURL,
          apiKey: config.apiKey,
          headers: config.headers
        });

        customModels.push({
          name: config.name,
          model: provider(config.modelId)
        });

        console.log(`Loaded custom provider: ${config.name}`);
      } catch (error) {
        console.warn(`Failed to create custom provider ${config.name}:`, error);
      }
    }
  } catch (error) {
    console.warn("Failed to load custom providers configuration:", error);
  }

  return customModels;
}

const baseModels: Model[] = [
  {
    name: "OpenAI GPT-5",
    model: "openai/gpt-5"
  },
  {
    name: "Anthropic Claude 4 Sonnet",
    model: "anthropic/claude-4-sonnet"
  },
  {
    name: "Google Gemini 2.5 Flash",
    model: "google/gemini-2.5-flash"
  },
  {
    name: "XAI Grok 4",
    model: "xai/grok-4"
  }
];

// Combine base models without custom providers
export const MODELS: Model[] = [...baseModels];
