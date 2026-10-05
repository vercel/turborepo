import { defineAgent, defineDynamic, type DynamicResolveContext } from "eve";

import {
  GPT_SOL_MODEL,
  selectPerformanceModels
} from "./lib/performance-models.js";
import { operatorModelSelection } from "./lib/operator-console.js";
import { sessionDate } from "./lib/repo.js";

function resolveModel(_event: unknown, ctx: DynamicResolveContext) {
  const operatorSelection = operatorModelSelection(
    ctx.session.auth.current,
    ctx.session.auth.initiator
  );
  if (operatorSelection) return operatorSelection;
  // Dynamic models have no compiled default; always return a concrete model.
  try {
    return selectPerformanceModels(sessionDate(ctx.session.id)).authorModel;
  } catch {
    return GPT_SOL_MODEL;
  }
}

export default defineAgent({
  model: defineDynamic({
    events: {
      "session.started": resolveModel,
      // Follow-up turns may change effort but omit the original model selection.
      "turn.started": resolveModel
    }
  })
});
