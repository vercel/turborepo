import { defineAgent } from "eve";

import { CLAUDE_OPUS_MODEL } from "../../lib/performance-models.js";

export default defineAgent({
  description:
    "Adversarially review GPT-authored Turborepo performance changes and return a structured verdict.",
  model: CLAUDE_OPUS_MODEL
});
