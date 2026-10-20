import { defineAgent } from "eve";

import { GPT_SOL_MODEL } from "../../lib/performance-models.js";

export default defineAgent({
  description:
    "Adversarially review Claude-authored Turborepo performance changes and return a structured verdict.",
  model: GPT_SOL_MODEL
});
