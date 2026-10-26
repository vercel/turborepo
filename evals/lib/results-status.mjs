import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

/** Invalid attempts and failures without a grader are not model failures, even
 * if the framework has cached their fingerprint (e.g. an AI Gateway 402). */
export function ungradedAttempts(resultsRoot, variants, fixtures) {
  const ungraded = [];
  for (const variant of variants) {
    const variantRoot = join(resultsRoot, variant);
    if (!existsSync(variantRoot)) continue;
    const runs = readdirSync(variantRoot)
      .filter((name) => statSync(join(variantRoot, name)).isDirectory())
      .sort()
      .toReversed();
    for (const fixture of fixtures) {
      const latest = runs.find((run) =>
        existsSync(join(variantRoot, run, fixture, "summary.json"))
      );
      if (!latest) continue;
      const fixtureRoot = join(variantRoot, latest, fixture);
      const attempts = readdirSync(fixtureRoot).filter((name) =>
        /^run-\d+$/.test(name)
      );
      if (
        attempts.some((attempt) => {
          const path = join(fixtureRoot, attempt, "result.json");
          if (!existsSync(path)) return false;
          const result = JSON.parse(readFileSync(path, "utf8"));
          return (
            typeof result.metadata?.invalidAttempt === "string" ||
            (result.status === "failed" && !result.outputPaths?.eval)
          );
        })
      ) {
        ungraded.push(`${variant}/${fixture}`);
      }
    }
  }
  return ungraded;
}
