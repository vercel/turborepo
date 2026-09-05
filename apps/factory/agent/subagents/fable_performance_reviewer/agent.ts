import { defineAgent } from "eve";

export default defineAgent({
  description:
    "Adversarially review GPT-authored Turborepo performance changes and return a structured verdict.",
  model: "anthropic/claude-fable-5.1"
});
