export interface CustomChatSettings {
  frequencyPenalty?: number;
  presencePenalty?: number;
  temperature?: number;
  topP?: number;
  maxTokens?: number;
  responseFormat?: {
    type: "text" | "json_object";
  };
  stop?: string[];
  user?: string;
}
