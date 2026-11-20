export const GPT_SOL_MODEL = "openai/gpt-6.1-sol";
export const CLAUDE_OPUS_MODEL = "anthropic/claude-opus-5.5";

export type PerformanceReviewer =
  | "fable_performance_reviewer"
  | "gpt_performance_reviewer";

export interface PerformanceModelSelection {
  authorModel: typeof GPT_SOL_MODEL | typeof CLAUDE_OPUS_MODEL;
  reviewerModel: typeof GPT_SOL_MODEL | typeof CLAUDE_OPUS_MODEL;
  reviewer: PerformanceReviewer;
}

export function selectPerformanceModels(date: Date): PerformanceModelSelection {
  if (Number.isNaN(date.getTime()))
    throw new Error("Invalid performance run date.");

  return date.getUTCDate() % 2 === 0
    ? {
        authorModel: GPT_SOL_MODEL,
        reviewerModel: CLAUDE_OPUS_MODEL,
        reviewer: "fable_performance_reviewer"
      }
    : {
        authorModel: CLAUDE_OPUS_MODEL,
        reviewerModel: GPT_SOL_MODEL,
        reviewer: "gpt_performance_reviewer"
      };
}
