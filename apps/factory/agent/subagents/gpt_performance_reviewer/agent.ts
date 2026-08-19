import { defineAgent } from "eve";

export default defineAgent({
  description:
    "Adversarially review Claude-authored Turborepo performance changes and return a structured verdict.",
  model: "openai/gpt-5.6-sol"
});
