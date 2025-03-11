"use client";

import { useState } from "react";
import type {
  BenchmarkBar,
  BenchmarkCategory,
  BenchmarkNumberOfModules,
} from "./pack-benchmarks";
import { BenchmarksGraph } from "./pack-benchmarks-graph";
import { PackBenchmarksPicker } from "./pack-benchmarks-picker";

export function DocsBenchmarksGraph(props: {
  bars: BenchmarkBar[];
  category: BenchmarkCategory;
}): JSX.Element {
  const [numberOfModules, setNumberOfModules] =
    useState<BenchmarkNumberOfModules>("1000");
  return (
    <div className="my-10">
      <BenchmarksGraph
        bars={props.bars}
        category={props.category}
        numberOfModules={numberOfModules}
        pinTime
      />
      <div className="mt-6 flex justify-center">
        <PackBenchmarksPicker setNumberOfModules={setNumberOfModules} />
      </div>
    </div>
  );
}
