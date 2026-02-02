/**
 * Types for cargo-llvm-cov JSON output and our storage format
 */

// Raw cargo-llvm-cov JSON output format
export interface LlvmCovReport {
  data: LlvmCovData[];
  type: string;
  version: string;
}

export interface LlvmCovData {
  files: LlvmCovFile[];
  functions: LlvmCovFunction[];
  totals: LlvmCovSummary;
}

export interface LlvmCovFile {
  filename: string;
  summary: LlvmCovSummary;
  segments: LlvmCovSegment[];
}

export interface LlvmCovFunction {
  name: string;
  count: number;
  regions: LlvmCovRegion[];
  filenames: string[];
}

export interface LlvmCovSummary {
  functions: LlvmCovCount;
  instantiations: LlvmCovCount;
  lines: LlvmCovCount;
  regions: LlvmCovCount;
  branches: LlvmCovCount;
}

export interface LlvmCovCount {
  count: number;
  covered: number;
  percent: number;
}

// Segment: [line, col, count, hasCount, isRegionEntry, isGapRegion]
export type LlvmCovSegment = [
  number,
  number,
  number,
  boolean,
  boolean,
  boolean
];

// Region: [lineStart, colStart, lineEnd, colEnd, executionCount, fileId, expandedFileId, kind]
export type LlvmCovRegion = [
  number,
  number,
  number,
  number,
  number,
  number,
  number,
  number
];

// Our processed/stored format
export interface CoverageReport {
  sha: string;
  branch: string;
  timestamp: string;
  summary: CoverageSummary;
  crates: CrateCoverage[];
  files: FileCoverage[];
}

export interface CoverageSummary {
  lines: CoverageMetric;
  functions: CoverageMetric;
  branches: CoverageMetric;
  regions: CoverageMetric;
}

export interface CoverageMetric {
  covered: number;
  total: number;
  percent: number;
}

export interface CrateCoverage {
  name: string;
  summary: CoverageSummary;
  files: string[];
}

export interface FileCoverage {
  path: string;
  crate: string;
  summary: CoverageSummary;
  uncoveredLines: number[];
}

// Commit entry for lists
export interface CommitEntry {
  sha: string;
  branch: string;
  timestamp: string;
  summary: CoverageSummary;
}

// API types
export interface UploadRequest {
  sha: string;
  branch: string;
  report: LlvmCovReport;
}

export interface UploadResponse {
  success: boolean;
  sha: string;
  summary: CoverageSummary;
  url: string;
}
