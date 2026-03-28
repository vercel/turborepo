/**
 * AI Agent Detection Utility
 *
 * Multi-signal detection for AI agents/bots. Used to serve markdown
 * responses when agents request docs pages.
 *
 * Three detection layers:
 * 1. Known UA patterns (definitive) — curated from https://bots.fyi/?tags=ai_assistant
 * 2. Signature-Agent header (definitive) — catches ChatGPT agent (RFC 9421)
 * 3. Missing browser fingerprint heuristic — catches unknown bots
 *
 * Optimizes for recall over precision: serving markdown to a non-AI bot
 * is low-harm; missing an AI agent means a worse experience.
 *
 * Last reviewed: 2026-03-20 against bots.fyi + official vendor docs
 */

// Layer 1: Known AI agent UA substrings (lowercase).
const AI_AGENT_UA_PATTERNS = [
  // Anthropic — https://support.claude.com/en/articles/8896518
  "claudebot",
  "claude-searchbot",
  "claude-user",
  "anthropic-ai",
  "claude-web",

  // OpenAI — https://platform.openai.com/docs/bots
  "chatgpt",
  "gptbot",
  "oai-searchbot",
  "openai",

  // Google AI
  "gemini",
  "bard",
  "google-cloudvertexbot",
  "google-extended",

  // Meta
  "meta-externalagent",
  "meta-externalfetcher",
  "meta-webindexer",

  // Search/Research AI
  "perplexity",
  "youbot",
  "you.com",
  "deepseekbot",

  // Coding assistants
  "cursor",
  "github-copilot",
  "codeium",
  "tabnine",
  "sourcegraph",

  // Other AI agents / data scrapers (low-harm to serve markdown)
  "cohere-ai",
  "bytespider",
  "amazonbot",
  "ai2bot",
  "diffbot",
  "omgili",
  "omgilibot",
];

// Layer 2: Known AI service URLs in Signature-Agent header (RFC 9421).
const SIGNATURE_AGENT_DOMAINS = ["chatgpt.com"];

// Layer 3: Traditional bot exclusion list — bots that should NOT trigger
// the heuristic layer (they're search engine crawlers, social previews, or
// monitoring tools, not AI agents).
const TRADITIONAL_BOT_PATTERNS = [
  "googlebot",
  "bingbot",
  "yandexbot",
  "baiduspider",
  "duckduckbot",
  "slurp",
  "msnbot",
  "facebot",
  "twitterbot",
  "linkedinbot",
  "whatsapp",
  "telegrambot",
  "pingdom",
  "uptimerobot",
  "newrelic",
  "datadog",
  "statuspage",
  "site24x7",
  "applebot",
];

// Broad regex for bot-like UA strings (used only in Layer 3 heuristic).
const BOT_LIKE_REGEX = /bot|agent|fetch|crawl|spider|search/i;

export type DetectionMethod = "ua-match" | "signature-agent" | "heuristic";

export interface DetectionResult {
  detected: boolean;
  method: DetectionMethod | null;
}

/**
 * Detects AI agents from HTTP request headers.
 *
 * Returns both whether the agent was detected and which signal triggered,
 * so callers can log the detection method for accuracy tracking.
 */
export function isAIAgent(request: {
  headers: { get(name: string): string | null };
}): DetectionResult {
  const userAgent = request.headers.get("user-agent");

  // Layer 1: Known UA pattern match
  if (userAgent) {
    const lowerUA = userAgent.toLowerCase();
    if (AI_AGENT_UA_PATTERNS.some((pattern) => lowerUA.includes(pattern))) {
      return { detected: true, method: "ua-match" };
    }
  }

  // Layer 2: Signature-Agent header (RFC 9421, used by ChatGPT agent)
  const signatureAgent = request.headers.get("signature-agent");
  if (signatureAgent) {
    const lowerSig = signatureAgent.toLowerCase();
    if (SIGNATURE_AGENT_DOMAINS.some((domain) => lowerSig.includes(domain))) {
      return { detected: true, method: "signature-agent" };
    }
  }

  // Layer 3: Missing browser fingerprint heuristic
  // Real browsers (Chrome 76+, Firefox 90+, Safari 16.4+) send sec-fetch-mode
  // on navigation requests. Its absence signals a programmatic client.
  const secFetchMode = request.headers.get("sec-fetch-mode");
  if (!secFetchMode && userAgent && BOT_LIKE_REGEX.test(userAgent)) {
    const lowerUA = userAgent.toLowerCase();
    const isTraditionalBot = TRADITIONAL_BOT_PATTERNS.some((pattern) =>
      lowerUA.includes(pattern)
    );
    if (!isTraditionalBot) {
      return { detected: true, method: "heuristic" };
    }
  }

  return { detected: false, method: null };
}

/**
 * Generates a markdown response for AI agents that hit non-existent URLs.
 */
export function generateAgentNotFoundResponse(requestedPath: string): string {
  return `# Page Not Found

The URL \`${requestedPath}\` does not exist in the documentation.

## How to find the correct page

1. **Browse the sitemap**: [/sitemap.md](/sitemap.md) — A structured index of all pages with URLs, content types, and descriptions
2. **Browse the full index**: [/llms.txt](/llms.txt) — Complete documentation index

## Tips for requesting documentation

- For markdown responses, append \`.md\` to URLs (e.g., \`/docs/getting-started.md\`)
- Use \`Accept: text/markdown\` header for content negotiation
`;
}
