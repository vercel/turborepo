import { inRepository, loadSelection, report } from "../selection";

/**
 * Jest's `filter` option: the test files jest found, without those the
 * selection drops. Jest calls it once for each project.
 */
async function filter(
  tests: string[]
): Promise<{ filtered: (string | { test: string })[] }> {
  const selection = loadSelection();
  const kept = tests.filter(
    (test) => !selection?.dropped.has(inRepository(selection, test))
  );
  if (selection) {
    report(selection, "jest", process.cwd(), kept.length, tests.length);
  }
  // Jest 30 reads the paths and jest 27 to 29 read the objects. Each ignores
  // the other form.
  return { filtered: [...kept, ...kept.map((test) => ({ test }))] };
}

export default filter;
