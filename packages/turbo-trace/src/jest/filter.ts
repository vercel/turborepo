import { inRepository, loadSelection, report } from "../selection";

type Filtered = { filtered: (string | { test: string })[] };
type Filter = ((tests: string[]) => Promise<Filtered>) & {
  setup?: () => unknown;
};

/** The `filter` the configuration had before `withTurboTrace`: it goes first. */
function userFilter(): Filter | undefined {
  const path = process.env.TURBO_TRACE_JEST_FILTER;
  return path ? require(path) : undefined;
}

/**
 * Jest's `filter` option: the test files jest found, without those the
 * selection drops. Jest calls it once for each project.
 */
async function filter(tests: string[]): Promise<Filtered> {
  let kept = tests;
  const user = userFilter();
  if (user) {
    const { filtered } = await user(tests);
    kept = filtered.map((entry) =>
      typeof entry === "string" ? entry : entry.test
    );
  }
  const selection = loadSelection();
  if (selection) {
    kept = kept.filter(
      (test) => !selection.dropped.has(inRepository(selection, test))
    );
    report(selection, "jest", process.cwd(), kept.length, tests.length);
  }
  // Jest 30 reads the paths and jest 27 to 29 read the objects. Each ignores
  // the other form.
  return { filtered: [...kept, ...kept.map((test) => ({ test }))] };
}

filter.setup = () => userFilter()?.setup?.();

export default filter;
