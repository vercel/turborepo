import { useState } from "react";
import {
  BenchmarkBar,
  BenchmarkCategory,
  BenchmarkNumberOfModules,
} from "./PackBenchmarks";
import { BenchmarksGraph } from "./PackBenchmarksGraph";
import { PackBenchmarksPicker } from "./PackBenchmarksPicker";

export function DocsBenchmarksGraph(props: {
  bars: BenchmarkBar[];
  category: BenchmarkCategory;
}) {
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
      <div className="flex justify-center mt-6">
        <PackBenchmarksPicker
          setNumberOfModules={setNumberOfModules}
        ></PackBenchmarksPicker>
      </div>
    </div>
  );
}
