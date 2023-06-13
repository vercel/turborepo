import minimist from "minimist";
import { setCurrentScenarios } from "./describe.js";
import { join } from "path";
import { Scenario, ScenarioVariant, runScenarios } from "./index.js";
import compose from "./interfaces/compose.js";

(async () => {
  const knownArgs = new Set([
    "scenario",
    "s",
    "json",
    "j",
    "console",
    "interactive",
    "i",
    "help",
    "h",
    "?",
    "_",
  ]);
  const args = minimist(process.argv.slice(2), {
    alias: {
      s: "scenario",
      j: "json",
      i: "interactive",
      "?": "help",
      h: "help",
    },
  });

  if (args.help || (Object.keys(args).length === 1 && args._.length === 0)) {
    console.log("Usage: devlow-bench [options] <scenario files>");
    console.log("## Selecting scenarios");
    console.log(
      "  --scenario=<filter>, -s=<filter>   Only run the scenario with the given name"
    );
    console.log(
      "  --<prop>=<value>                   Filter by any variant property defined in scenarios"
    );
    console.log("## Output");
    console.log(
      "  --json=<path>, -j=<path>           Write the results to the given path as JSON"
    );
    console.log(
      "  --console                          Print the results to the console"
    );
    console.log(
      "  --interactive, -i                  Select scenarios and variants interactively"
    );
    console.log("## Help");
    console.log("  --help, -h, -?                     Show this help");
  }

  const scenarios: Scenario[] = [];
  setCurrentScenarios(scenarios);

  for (const path of args._) {
    await import(join(process.cwd(), path));
  }

  setCurrentScenarios(null);

  const cliIface = {
    filterScenarios: async (scenarios: Scenario[]) => {
      if (args.scenario) {
        const filter = [].concat(args.scenario);
        return scenarios.filter((s) =>
          filter.some((filter) => s.name.includes(filter))
        );
      }
      return scenarios;
    },
    filterScenarioVariants: async (variants: ScenarioVariant[]) => {
      const propEntries = Object.entries(args).filter(
        ([key]) => !knownArgs.has(key)
      );
      if (propEntries.length === 0) return variants;
      for (const [key, value] of propEntries) {
        const values = (Array.isArray(value) ? value : [value]).map((v) =>
          v.toString()
        );
        variants = variants.filter((variant) => {
          const prop = variant.props[key];
          if (typeof prop === "undefined") return false;
          const str = prop.toString();
          return values.some((v) => str.includes(v));
        });
      }
      return variants;
    },
  };
  let ifaces = [
    cliIface,
    args.interactive && (await import("./interfaces/interactive.js")).default(),
    args.json && (await import("./interfaces/json.js")).default(args.json),
    args.console !== false &&
      (await import("./interfaces/console.js")).default(),
  ].filter((x) => x);
  await runScenarios(scenarios, compose(...ifaces));
})().catch((e) => {
  console.error(e.stack);
});
