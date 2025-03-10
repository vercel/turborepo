import type { BenchmarkNumberOfModules } from "./pack-benchmarks";
import { PackDropdown } from "./pack-dropdown";

export function PackBenchmarksPicker(props: {
  setNumberOfModules: (num: BenchmarkNumberOfModules) => void;
}): JSX.Element {
  return (
    <div className="flex items-center gap-3">
      <a
        className="text-sm  text-[#666666] underline-offset-4 hover:underline dark:text-[#888888]"
        href="https://github.com/vercel/turbo/blob/main/docs/components/pages/pack-home/benchmark-data"
      >
        React Components
      </a>
      <PackDropdown
        onOptionSelected={(value) => {
          props.setNumberOfModules(value);
        }}
      />
    </div>
  );
}
