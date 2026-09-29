import { defineAgent, defineDynamic } from "eve";

import {
  GPT_SOL_MODEL,
  selectPerformanceModels
} from "./lib/performance-models.js";
import { selectedOperatorModel } from "./lib/operator-console.js";
import { sessionDate } from "./lib/repo.js";

export default defineAgent({
  build: {
    // Harness adapters load sandbox bootstrap assets relative to import.meta.url.
    // Preserve their package layout in Eve's hosted output.
    externalDependencies: [
      "@ai-sdk/harness-acp",
      "@ai-sdk/harness-claude-code",
      "@ai-sdk/harness-codex",
      "@ai-sdk/harness-opencode"
    ]
  },
  model: defineDynamic({
    events: {
      // A dynamic model has no compiled default and a resolver that throws
      // fails the turn, so keep supplying the model the removed `fallback`
      // option used to cover when a session id cannot be parsed.
      "session.started": (_event, ctx) => {
        const operatorModel = selectedOperatorModel(ctx.session.auth.current);
        if (operatorModel) return operatorModel;
        try {
          return selectPerformanceModels(sessionDate(ctx.session.id))
            .authorModel;
        } catch {
          return GPT_SOL_MODEL;
        }
      }
    }
  })
});
