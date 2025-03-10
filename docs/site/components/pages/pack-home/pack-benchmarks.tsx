export type BenchmarkNumberOfModules = "1000" | "5000" | "10000" | "30000";
export type BenchmarkCategory =
  | "cold"
  | "from_cache"
  | "file_change"
  | "code_build"
  | "build_from_cache";
export interface BenchmarkData {
  next13: number;
  next12: number;
  vite: number;
  next11: number;
}

export interface BenchmarkBar {
  label: string;
  version: string;
  key: keyof BenchmarkData;
  turbo?: true;
  swc?: true;
}
