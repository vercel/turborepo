import { useState } from "react";
import { PackBenchmarkTabs } from "./PackBenchmarkTabs";
import { SectionHeader, SectionSubtext } from "../home-shared/Headings";
import { BenchmarksGraph } from "./PackBenchmarksGraph";
import { PackDropdown } from "./PackDropdown";
import { FadeIn } from "../home-shared/FadeIn";

export type BenchmarkNumberOfModules = "1000" | "5000" | "10000" | "30000";
export type BenchmarkCategory =
  | "cold"
  | "from_cache"
  | "file_change"
  | "code_build"
  | "build_from_cache";

export function PackBenchmarks() {
  const [numberOfModules, setNumberOfModules] =
    useState<BenchmarkNumberOfModules>("1000");
  const [category, setCategory] = useState<BenchmarkCategory>("cold");

  return (
    <FadeIn className="font-sans relative py-16 md:py-24 lg:py-32 w-full items-center flex flex-col gap-10 justify-center">
      <div className="flex flex-col gap-5 md:gap-6 items-center">
        <SectionHeader>Faster Than Fast</SectionHeader>
        <SectionSubtext>
          Crafted by the creators of Webpack, Turbopack delivers unparalleled
          performance at scale.
        </SectionSubtext>
      </div>
      <div className="flex flex-col w-full items-center">
        <PackBenchmarkTabs onTabChange={setCategory} />
        <BenchmarksGraph
          category={category}
          numberOfModules={numberOfModules}
        />
      </div>
      <div className="flex gap-3 items-center">
        <a
          className="dark:text-[#888888]  hover:underline underline-offset-4 text-[#666666] text-sm"
          href="https://github.com/vercel/turbo/blob/main/docs/components/pages/pack-home/benchmark-data"
        >
          React Components
        </a>
        <PackDropdown onOptionSelected={(value) => setNumberOfModules(value)} />
      </div>
    </FadeIn>
  );
}
