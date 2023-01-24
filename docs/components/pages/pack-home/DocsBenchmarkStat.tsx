import benchmarkData from "./benchmark-data/data.json";

type StatFunc = (data: typeof benchmarkData) => string;

/**
 * Replace with satisfies keyword when TS 4.9 drops
 */
const satisfies =
  <T,>() =>
  <U extends T>(t: U) =>
    t;

const formatToSeconds = (seconds: number) => `${seconds.toFixed(1)}s`;
const formatPercentage = (percentage: number) => `${percentage.toFixed(1)}x`;

const stats = satisfies<Record<string, StatFunc>>()({
  "next12-cold-1000": (data) => formatToSeconds(data.cold[1000].next12),
  "turbopack-cold-1000": (data) => formatToSeconds(data.cold[1000].next13),
  "turbopack-cold-vs-next12": (data) =>
    formatPercentage(data.cold[1000].next12 / data.cold[1000].next13),
  "turbopack-cold-vs-next12-30000": (data) =>
    formatPercentage(data.cold[30000].next12 / data.cold[30000].next13),
  "turbopack-update-vs-next12": (data) =>
    formatPercentage(
      data.file_change[1000].next12 / data.file_change[1000].next13
    ),
  "turbopack-update-vs-next12-30000": (data) =>
    formatPercentage(
      data.file_change[30000].next12 / data.file_change[30000].next13
    ),
  "vite-cold-1000": (data) => formatToSeconds(data.cold[1000].vite),
  "turbopack-cold-vs-vite": (data) =>
    formatPercentage(data.cold[1000].vite / data.cold[1000].next13),
  "turbopack-cold-vs-vite-30000": (data) =>
    formatPercentage(data.cold[30000].vite / data.cold[30000].next13),
  "turbopack-update-vs-vite": (data) =>
    formatPercentage(
      data.file_change[1000].vite / data.file_change[1000].next13
    ),
  "turbopack-update-vs-vite-30000": (data) =>
    formatPercentage(
      data.file_change[30000].vite / data.file_change[30000].next13
    ),
});

type Stat = keyof typeof stats;

export function DocsBenchmarkStat(props: { stat: Stat }) {
  if (!stats[props.stat]) {
    throw new Error(`Invalid stat: ${props.stat}`);
  }
  return stats[props.stat](benchmarkData);
}
