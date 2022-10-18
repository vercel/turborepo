import { useState } from "react";
import { PackBenchmarkTabs } from "./PackBenchmarkTabs";
import { SectionHeader, SectionSubtext } from "./Headings";
import { BenchmarksGraph } from "./PackBenchmarksGraph";
import { GradientSectionBorder } from "./GradientSectionBorder";
import { PackDropdown } from "./PackDropdown";
import { FadeIn } from "./FadeIn";

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
      <div className="flex flex-col gap-5 md:gap-6">
        <SectionHeader>Faster Than Fast</SectionHeader>
        <SectionSubtext>
          Crafted by the creators of Webpack, Turbopack delivers unparalleled
        </SectionSubtext>
      </div>
      <PackBenchmarkTabs onTabChange={setCategory} />
      <BenchmarksGraph category={category} numberOfModules={numberOfModules} />
      <div className="flex gap-3 items-center">
        <p className="dark:text-[#888888] text-[#666666] text-sm">
          React Components
        </p>
        <PackDropdown onOptionSelected={(value) => setNumberOfModules(value)} />
      </div>
    </FadeIn>
  );
}
