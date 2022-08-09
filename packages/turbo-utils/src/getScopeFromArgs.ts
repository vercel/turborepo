import type { Scope } from "./types";

function getScopeFromArgs({ args }: { args: Array<string> }): Scope {
  if (args.length && args[0] != null) {
    return { scope: args[0], context: {} };
  }
  return { scope: null, context: {} };
}

export default getScopeFromArgs;
