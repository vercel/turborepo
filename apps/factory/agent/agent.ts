import { defineAgent, defineDynamic } from "eve";

import {
  GPT_SOL_MODEL,
  selectPerformanceModels
} from "./lib/performance-models.js";
import { selectedOperatorModel } from "./lib/operator-console.js";
import { sessionDate } from "./lib/repo.js";

export default defineAgent({
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
