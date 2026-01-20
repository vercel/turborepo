"use client";

import { useState, useEffect } from "react";
import {
  ArrowUpDown,
  Info,
  Search,
  ChevronDown,
  ChevronRight
} from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow
} from "@/components/ui/table";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger
} from "@/components/ui/tooltip";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue
} from "@/components/ui/select";
import evaluationsJson from "@/app/data/evaluations.json";

// Type definitions for evaluation data
interface EvaluationResult {
  projectName: string;
  experimentName: string;
  projectId: string;
  experimentId: string;
  projectUrl: string;
  experimentUrl: string;
  comparisonExperimentName: string;
  scores: {
    eval_score: {
      name: string;
      score: number;
      improvements: number;
      regressions: number;
      diff?: number;
    };
    build_score?: {
      name: string;
      score: number;
      improvements: number;
      regressions: number;
      diff?: number;
    };
    lint_score?: {
      name: string;
      score: number;
      improvements: number;
      regressions: number;
      diff?: number;
    };
    test_score?: {
      name: string;
      score: number;
      improvements: number;
      regressions: number;
      diff?: number;
    };
  };
  metrics: {
    start: {
      name: string;
      metric: number;
      unit: string;
      improvements: number;
      regressions: number;
    };
    end: {
      name: string;
      metric: number;
      unit: string;
      improvements: number;
      regressions: number;
    };
    duration: {
      name: string;
      metric: number;
      unit: string;
      improvements: number;
      regressions: number;
      diff?: number;
    };
    prompt_tokens: {
      name: string;
      metric: number;
      unit: string;
      improvements: number;
      regressions: number;
      diff?: number;
    };
    completion_tokens: {
      name: string;
      metric: number;
      unit: string;
      improvements: number;
      regressions: number;
      diff?: number;
    };
    total_tokens: {
      name: string;
      metric: number;
      unit: string;
      improvements: number;
      regressions: number;
      diff?: number;
    };
    prompt_cached_tokens: {
      name: string;
      metric: number;
      unit: string;
      improvements: number;
      regressions: number;
      diff?: number;
    };
    prompt_cache_creation_tokens: {
      name: string;
      metric: number;
      unit: string;
      improvements: number;
      regressions: number;
      diff?: number;
    };
  };
}

interface EvaluationItem {
  status: string;
  value: {
    evalPath: string;
    status: string;
    result: EvaluationResult | EvaluationResult[];
  };
}

interface ProcessedEvaluation {
  evaluationId: string;
  evaluationType: string;
  modelName: string;
  projectName: string;
  experimentName: string;
  projectId: string;
  experimentId: string;
  projectUrl: string;
  experimentUrl: string;
  scores: {
    evaluationScore: {
      name: string;
      score: number;
      improvements: number;
      regressions: number;
    };
    buildScore?: {
      name: string;
      score: number;
      improvements: number;
      regressions: number;
    };
    lintScore?: {
      name: string;
      score: number;
      improvements: number;
      regressions: number;
    };
    testScore?: {
      name: string;
      score: number;
      improvements: number;
      regressions: number;
    };
  };
  metrics: {
    duration: {
      name: string;
      metric: number;
      unit: string;
      improvements: number;
      regressions: number;
    };
    promptTokens: {
      name: string;
      metric: number;
      unit: string;
      improvements: number;
      regressions: number;
    };
    completionTokens: {
      name: string;
      metric: number;
      unit: string;
      improvements: number;
      regressions: number;
    };
    totalTokens: {
      name: string;
      metric: number;
      unit: string;
      improvements: number;
      regressions: number;
    };
  };
}

// Process the evaluation data from JSON
const processEvaluationData = (): ProcessedEvaluation[] => {
  const evaluations: ProcessedEvaluation[] = [];

  (evaluationsJson as EvaluationItem[]).forEach((item) => {
    if (item.status === "fulfilled" && item.value.status === "success") {
      const results = Array.isArray(item.value.result)
        ? item.value.result
        : [item.value.result];

      results.forEach((result) => {
        const modelName = result.experimentName.split("-")[0].trim();

        evaluations.push({
          evaluationId: result.experimentId,
          evaluationType: item.value.evalPath,
          modelName: modelName,
          projectName: result.projectName,
          experimentName: result.experimentName,
          projectId: result.projectId,
          experimentId: result.experimentId,
          projectUrl: result.projectUrl,
          experimentUrl: result.experimentUrl,
          scores: {
            evaluationScore: {
              name: "eval_score",
              score: result.scores.eval_score.score,
              improvements: result.scores.eval_score.improvements,
              regressions: result.scores.eval_score.regressions
            },
            buildScore: result.scores.build_score
              ? {
                  name: "build_score",
                  score: result.scores.build_score.score,
                  improvements: result.scores.build_score.improvements,
                  regressions: result.scores.build_score.regressions
                }
              : undefined,
            lintScore: result.scores.lint_score
              ? {
                  name: "lint_score",
                  score: result.scores.lint_score.score,
                  improvements: result.scores.lint_score.improvements,
                  regressions: result.scores.lint_score.regressions
                }
              : undefined,
            testScore: result.scores.test_score
              ? {
                  name: "test_score",
                  score: result.scores.test_score.score,
                  improvements: result.scores.test_score.improvements,
                  regressions: result.scores.test_score.regressions
                }
              : undefined
          },
          metrics: {
            duration: {
              name: "duration",
              metric: result.metrics.duration.metric,
              unit: "s",
              improvements: result.metrics.duration.improvements,
              regressions: result.metrics.duration.regressions
            },
            promptTokens: {
              name: "prompt_tokens",
              metric: result.metrics.prompt_tokens.metric,
              unit: "tok",
              improvements: result.metrics.prompt_tokens.improvements,
              regressions: result.metrics.prompt_tokens.regressions
            },
            completionTokens: {
              name: "completion_tokens",
              metric: result.metrics.completion_tokens.metric,
              unit: "tok",
              improvements: result.metrics.completion_tokens.improvements,
              regressions: result.metrics.completion_tokens.regressions
            },
            totalTokens: {
              name: "total_tokens",
              metric: result.metrics.total_tokens.metric,
              unit: "tok",
              improvements: result.metrics.total_tokens.improvements,
              regressions: result.metrics.total_tokens.regressions
            }
          }
        });
      });
    }
  });

  return evaluations;
};

const evaluationData = processEvaluationData();
const allModels = Array.from(
  new Set(evaluationData.map((item) => item.modelName))
);

export function ComparisonTable() {
  const [sortColumn, setSortColumn] = useState("avgScore");
  const [sortDirection, setSortDirection] = useState("desc");
  const [searchTerm, setSearchTerm] = useState("");
  const [expandedModels, setExpandedModels] = useState<Set<string>>(new Set());
  const [comparisonSelections, setComparisonSelections] = useState<
    Record<string, string>
  >({});
  const [globalComparison, setGlobalComparison] = useState(allModels[0]); // Updated default value

  const handleSort = (column: string) => {
    if (sortColumn === column) {
      setSortDirection(sortDirection === "asc" ? "desc" : "asc");
    } else {
      setSortColumn(column);
      setSortDirection("desc");
    }
  };

  const toggleModelExpansion = (modelName: string) => {
    const newExpanded = new Set(expandedModels);
    if (newExpanded.has(modelName)) {
      newExpanded.delete(modelName);
    } else {
      newExpanded.add(modelName);
    }
    setExpandedModels(newExpanded);
  };

  const handleComparisonChange = (
    evaluationId: string,
    comparisonModel: string
  ) => {
    setComparisonSelections((prev) => ({
      ...prev,
      [evaluationId]: comparisonModel
    }));
  };

  const handleGlobalComparisonChange = (comparisonModel: string) => {
    setGlobalComparison(comparisonModel);
  };

  const getComparisonData = (
    evaluation: ProcessedEvaluation,
    comparisonModelName: string
  ) => {
    if (!comparisonModelName) return null;

    // Find the comparison evaluation data for the same evaluation type but different model
    const comparisonEvaluation = evaluationData.find(
      (item) =>
        item.evaluationType === evaluation.evaluationType &&
        item.modelName === comparisonModelName
    );

    if (!comparisonEvaluation) return null;

    const scoreDiff =
      evaluation.scores.evaluationScore.score -
      comparisonEvaluation.scores.evaluationScore.score;
    const durationDiff =
      evaluation.metrics.duration.metric -
      comparisonEvaluation.metrics.duration.metric;
    const tokenDiff =
      evaluation.metrics.totalTokens.metric -
      comparisonEvaluation.metrics.totalTokens.metric;

    return {
      scoreDiff: scoreDiff.toFixed(3),
      durationDiff: durationDiff.toFixed(2),
      tokenDiff: Math.round(tokenDiff),
      comparisonScore: comparisonEvaluation.scores.evaluationScore.score,
      comparisonDuration: comparisonEvaluation.metrics.duration.metric,
      comparisonTokens: comparisonEvaluation.metrics.totalTokens.metric
    };
  };

  const getModelComparisonData = (
    modelSummary: ModelSummary,
    comparisonModelName: string
  ) => {
    if (!comparisonModelName) return null;

    const comparisonSummary = modelSummaries.find(
      (summary) => summary.modelName === comparisonModelName
    );
    if (!comparisonSummary) return null;

    const scoreDiff = modelSummary.avgScore - comparisonSummary.avgScore;
    const durationDiff =
      modelSummary.avgDuration - comparisonSummary.avgDuration;
    const tokenDiff = modelSummary.avgTokens - comparisonSummary.avgTokens;

    return {
      scoreDiff: scoreDiff.toFixed(3),
      durationDiff: durationDiff.toFixed(2),
      tokenDiff: Math.round(tokenDiff),
      comparisonScore: comparisonSummary.avgScore,
      comparisonDuration: comparisonSummary.avgDuration,
      comparisonTokens: comparisonSummary.avgTokens
    };
  };

  // Group evaluations by model
  const groupedData = evaluationData.reduce<
    Record<string, ProcessedEvaluation[]>
  >((acc, item) => {
    if (!acc[item.modelName]) {
      acc[item.modelName] = [];
    }
    acc[item.modelName].push(item);
    return acc;
  }, {});

  interface ModelSummary {
    modelName: string;
    evaluations: ProcessedEvaluation[];
    avgScore: number;
    avgBuildScore: number;
    avgLintScore: number;
    avgTestScore: number;
    avgDuration: number;
    avgTokens: number;
    totalEvaluations: number;
  }

  // Calculate aggregate metrics for each model
  const modelSummaries: ModelSummary[] = Object.entries(groupedData).map(
    ([modelName, evaluations]) => {
      const avgScore =
        evaluations.reduce(
          (sum, evaluationItem) =>
            sum + evaluationItem.scores.evaluationScore.score,
          0
        ) / evaluations.length;
      const avgBuildScore =
        evaluations.reduce(
          (sum, evaluationItem) =>
            sum + (evaluationItem.scores.buildScore?.score ?? 0),
          0
        ) / evaluations.length;
      const avgLintScore =
        evaluations.reduce(
          (sum, evaluationItem) =>
            sum + (evaluationItem.scores.lintScore?.score ?? 0),
          0
        ) / evaluations.length;
      const avgTestScore =
        evaluations.reduce(
          (sum, evaluationItem) =>
            sum + (evaluationItem.scores.testScore?.score ?? 0),
          0
        ) / evaluations.length;
      const avgDuration =
        evaluations.reduce(
          (sum, evaluationItem) => sum + evaluationItem.metrics.duration.metric,
          0
        ) / evaluations.length;
      const avgTokens =
        evaluations.reduce(
          (sum, evaluationItem) =>
            sum + evaluationItem.metrics.totalTokens.metric,
          0
        ) / evaluations.length;
      const totalEvaluations = evaluations.length;

      return {
        modelName,
        evaluations,
        avgScore,
        avgBuildScore,
        avgLintScore,
        avgTestScore,
        avgDuration,
        avgTokens,
        totalEvaluations
      };
    }
  );

  // Filter and sort model summaries
  const filteredSummaries = modelSummaries
    .filter((summary) => {
      const matchesSearch =
        summary.modelName.toLowerCase().includes(searchTerm.toLowerCase()) ||
        summary.evaluations.some((evaluation) =>
          evaluation.evaluationType
            .toLowerCase()
            .includes(searchTerm.toLowerCase())
        );
      return matchesSearch;
    })
    .sort((a, b) => {
      let aValue: string | number = 0;
      let bValue: string | number = 0;

      if (sortColumn === "modelName") {
        aValue = a.modelName;
        bValue = b.modelName;
      } else if (sortColumn === "totalEvaluations") {
        aValue = a.totalEvaluations;
        bValue = b.totalEvaluations;
      } else if (sortColumn === "avgScore") {
        aValue = a.avgScore;
        bValue = b.avgScore;
      } else if (sortColumn === "avgBuildScore") {
        aValue = a.avgBuildScore;
        bValue = b.avgBuildScore;
      } else if (sortColumn === "avgLintScore") {
        aValue = a.avgLintScore;
        bValue = b.avgLintScore;
      } else if (sortColumn === "avgTestScore") {
        aValue = a.avgTestScore;
        bValue = b.avgTestScore;
      } else if (sortColumn === "avgDuration") {
        aValue = a.avgDuration;
        bValue = b.avgDuration;
      } else if (sortColumn === "avgTokens") {
        aValue = a.avgTokens;
        bValue = b.avgTokens;
      }

      if (sortDirection === "asc") {
        return aValue > bValue ? 1 : -1;
      } else {
        return aValue < bValue ? 1 : -1;
      }
    });

  const getDiffColor = (diff: string, isLowerBetter = false) => {
    const numDiff = Number.parseFloat(diff);
    if (numDiff === 0) return "text-gray-500";
    if ((numDiff > 0 && !isLowerBetter) || (numDiff < 0 && isLowerBetter)) {
      return "text-green-600";
    }
    return "text-red-600";
  };

  const formatDiff = (diff: string) => {
    const numDiff = Number.parseFloat(diff);
    if (numDiff > 0) return `+${diff}`;
    return diff;
  };

  const getScoreColor = (score: number) => {
    if (score >= 0.9) return "text-green-600";
    if (score >= 0.8) return "text-emerald-600";
    if (score >= 0.7) return "text-yellow-600";
    return "text-red-600";
  };

  return (
    <TooltipProvider>
      <div className="space-y-4">
        <div className="flex flex-col sm:flex-row gap-4">
          <div className="relative flex-1">
            <Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
            <Input
              type="search"
              placeholder="Search models or evaluations..."
              className="pl-8"
              value={searchTerm}
              onChange={(e) => setSearchTerm(e.target.value)}
            />
          </div>
          <Select
            value={globalComparison}
            onValueChange={handleGlobalComparisonChange}
          >
            <SelectTrigger className="w-[200px]">
              <SelectValue placeholder="Compare all models to..." />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={allModels[0]}>No comparison</SelectItem>{" "}
              {/* Updated value */}
              {allModels.map((model) => (
                <SelectItem key={model} value={model}>
                  vs {model}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Button
            variant="outline"
            onClick={() => {
              if (expandedModels.size === filteredSummaries.length) {
                setExpandedModels(new Set());
              } else {
                setExpandedModels(
                  new Set(filteredSummaries.map((s) => s.modelName))
                );
              }
            }}
          >
            {expandedModels.size === filteredSummaries.length
              ? "Collapse All"
              : "Expand All"}
          </Button>
        </div>

        <div className="rounded-md border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead
                  className="w-[300px] cursor-pointer"
                  onClick={() => handleSort("modelName")}
                >
                  <div className="flex items-center gap-1 font-bold">
                    Model
                    <ArrowUpDown className="h-4 w-4" />
                  </div>
                </TableHead>
                <TableHead
                  className="cursor-pointer"
                  onClick={() => handleSort("totalEvaluations")}
                >
                  <div className="flex items-center gap-1 font-bold">
                    Evaluations
                    <ArrowUpDown className="h-4 w-4" />
                  </div>
                </TableHead>
                <TableHead
                  className="cursor-pointer"
                  onClick={() => handleSort("avgScore")}
                >
                  <div className="flex items-center gap-1 font-bold">
                    Overall
                    <ArrowUpDown className="h-4 w-4" />
                  </div>
                </TableHead>
                <TableHead
                  className="cursor-pointer"
                  onClick={() => handleSort("avgBuildScore")}
                >
                  <div className="flex items-center gap-1 font-bold">
                    Build
                    <ArrowUpDown className="h-4 w-4" />
                  </div>
                </TableHead>
                <TableHead
                  className="cursor-pointer"
                  onClick={() => handleSort("avgLintScore")}
                >
                  <div className="flex items-center gap-1 font-bold">
                    Lint
                    <ArrowUpDown className="h-4 w-4" />
                  </div>
                </TableHead>
                <TableHead
                  className="cursor-pointer"
                  onClick={() => handleSort("avgTestScore")}
                >
                  <div className="flex items-center gap-1 font-bold">
                    Test
                    <ArrowUpDown className="h-4 w-4" />
                  </div>
                </TableHead>
                <TableHead
                  className="cursor-pointer"
                  onClick={() => handleSort("avgDuration")}
                >
                  <div className="flex items-center gap-1 font-bold">
                    Avg Duration (s)
                    <ArrowUpDown className="h-4 w-4" />
                  </div>
                </TableHead>
                <TableHead
                  className="cursor-pointer"
                  onClick={() => handleSort("avgTokens")}
                >
                  <div className="flex items-center gap-1 font-bold">
                    Avg Tokens
                    <ArrowUpDown className="h-4 w-4" />
                  </div>
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {filteredSummaries.map((summary) => {
                const modelComparisonData =
                  globalComparison && summary.modelName !== globalComparison
                    ? getModelComparisonData(summary, globalComparison)
                    : null;

                return (
                  <>
                    {/* Model Summary Row */}
                    <TableRow
                      key={summary.modelName}
                      className="cursor-pointer hover:bg-muted/50"
                      onClick={() => toggleModelExpansion(summary.modelName)}
                    >
                      <TableCell className="font-medium">
                        <div className="flex items-center gap-2">
                          {expandedModels.has(summary.modelName) ? (
                            <ChevronDown className="h-4 w-4" />
                          ) : (
                            <ChevronRight className="h-4 w-4" />
                          )}
                          {summary.modelName}
                          {globalComparison &&
                            summary.modelName === globalComparison && (
                              <Badge variant="outline" className="text-xs">
                                baseline
                              </Badge>
                            )}
                        </div>
                      </TableCell>
                      <TableCell>
                        <Badge variant="secondary">
                          {summary.totalEvaluations}
                        </Badge>
                      </TableCell>
                      <TableCell>
                        <div className="flex items-center gap-2">
                          <span
                            className={`font-semibold ${getScoreColor(summary.avgScore)}`}
                          >
                            {summary.avgScore.toFixed(3)}
                          </span>
                          {modelComparisonData && (
                            <>
                              <span
                                className={getDiffColor(
                                  modelComparisonData.scoreDiff
                                )}
                              >
                                ({formatDiff(modelComparisonData.scoreDiff)})
                              </span>
                              <Tooltip>
                                <TooltipTrigger>
                                  <Info className="h-4 w-4 text-muted-foreground" />
                                </TooltipTrigger>
                                <TooltipContent>
                                  <p>Compared to {globalComparison}</p>
                                  <p>
                                    Baseline score:{" "}
                                    {modelComparisonData.comparisonScore.toFixed(
                                      3
                                    )}
                                  </p>
                                </TooltipContent>
                              </Tooltip>
                            </>
                          )}
                        </div>
                      </TableCell>
                      <TableCell>
                        <div className="flex items-center gap-2">
                          <span
                            className={`font-semibold ${getScoreColor(summary.avgBuildScore)}`}
                          >
                            {summary.avgBuildScore.toFixed(3)}
                          </span>
                        </div>
                      </TableCell>
                      <TableCell>
                        <div className="flex items-center gap-2">
                          <span
                            className={`font-semibold ${getScoreColor(summary.avgLintScore)}`}
                          >
                            {summary.avgLintScore.toFixed(3)}
                          </span>
                        </div>
                      </TableCell>
                      <TableCell>
                        <div className="flex items-center gap-2">
                          <span
                            className={`font-semibold ${getScoreColor(summary.avgTestScore)}`}
                          >
                            {summary.avgTestScore.toFixed(3)}
                          </span>
                        </div>
                      </TableCell>
                      <TableCell>
                        <div className="flex items-center gap-2">
                          <span>{summary.avgDuration.toFixed(2)}</span>
                          {modelComparisonData && (
                            <span
                              className={getDiffColor(
                                modelComparisonData.durationDiff,
                                true
                              )}
                            >
                              ({formatDiff(modelComparisonData.durationDiff)})
                            </span>
                          )}
                        </div>
                      </TableCell>
                      <TableCell>
                        <div className="flex items-center gap-2">
                          <span>
                            {Math.round(summary.avgTokens).toLocaleString()}
                          </span>
                          {modelComparisonData && (
                            <span
                              className={getDiffColor(
                                modelComparisonData.tokenDiff.toString(),
                                true
                              )}
                            >
                              (
                              {formatDiff(
                                modelComparisonData.tokenDiff.toLocaleString()
                              )}
                              )
                            </span>
                          )}
                        </div>
                      </TableCell>
                    </TableRow>

                    {/* Individual Evaluation Rows */}
                    {expandedModels.has(summary.modelName) &&
                      summary.evaluations.map((evaluation) => {
                        const selectedComparison =
                          comparisonSelections[evaluation.evaluationId] ||
                          globalComparison;
                        const comparisonData =
                          selectedComparison &&
                          selectedComparison !== evaluation.modelName
                            ? getComparisonData(evaluation, selectedComparison)
                            : null;
                        const availableModels = allModels.filter(
                          (model) => model !== evaluation.modelName
                        );

                        return (
                          <TableRow
                            key={evaluation.experimentId}
                            className="bg-muted/20"
                          >
                            <TableCell className="pl-8">
                              <div className="text-sm">
                                <a
                                  href={evaluation.experimentUrl}
                                  target="_blank"
                                  rel="noopener noreferrer"
                                  className="text-blue-600 hover:text-blue-800 hover:underline transition-colors"
                                >
                                  {evaluation.evaluationType}
                                </a>
                              </div>
                            </TableCell>
                            <TableCell>
                              <Select
                                value={
                                  comparisonSelections[
                                    evaluation.evaluationId
                                  ] || allModels[0]
                                } // Updated value
                                onValueChange={(value) =>
                                  handleComparisonChange(
                                    evaluation.evaluationId,
                                    value
                                  )
                                }
                              >
                                <SelectTrigger className="w-[180px]">
                                  <SelectValue
                                    placeholder={
                                      globalComparison
                                        ? `vs ${globalComparison}`
                                        : "Select comparison"
                                    }
                                  />
                                </SelectTrigger>
                                <SelectContent>
                                  <SelectItem value={allModels[0]}>
                                    {globalComparison
                                      ? `Use global (vs ${globalComparison})`
                                      : "No comparison"}
                                  </SelectItem>
                                  {availableModels.map((model) => (
                                    <SelectItem key={model} value={model}>
                                      vs {model}
                                    </SelectItem>
                                  ))}
                                </SelectContent>
                              </Select>
                            </TableCell>
                            <TableCell>
                              <div className="flex items-center gap-2">
                                <span
                                  className={`font-semibold ${getScoreColor(evaluation.scores.evaluationScore.score)}`}
                                >
                                  {evaluation.scores.evaluationScore.score.toFixed(
                                    3
                                  )}
                                </span>
                                {comparisonData && (
                                  <span
                                    className={getDiffColor(
                                      comparisonData.scoreDiff
                                    )}
                                  >
                                    ({formatDiff(comparisonData.scoreDiff)})
                                  </span>
                                )}
                                <Tooltip>
                                  <TooltipTrigger>
                                    <Info className="h-4 w-4 text-muted-foreground" />
                                  </TooltipTrigger>
                                  <TooltipContent>
                                    <p>Evaluation score (higher is better)</p>
                                    {comparisonData && (
                                      <p>
                                        Comparison score:{" "}
                                        {comparisonData.comparisonScore.toFixed(
                                          3
                                        )}
                                      </p>
                                    )}
                                    <p>
                                      Improvements:{" "}
                                      {
                                        evaluation.scores.evaluationScore
                                          .improvements
                                      }
                                    </p>
                                    <p>
                                      Regressions:{" "}
                                      {
                                        evaluation.scores.evaluationScore
                                          .regressions
                                      }
                                    </p>
                                  </TooltipContent>
                                </Tooltip>
                              </div>
                            </TableCell>
                            <TableCell>
                              <div className="flex items-center gap-2">
                                <span
                                  className={`font-semibold ${getScoreColor(evaluation.scores.buildScore?.score ?? 0)}`}
                                >
                                  {(
                                    evaluation.scores.buildScore?.score ?? 0
                                  ).toFixed(3)}
                                </span>
                              </div>
                            </TableCell>
                            <TableCell>
                              <div className="flex items-center gap-2">
                                <span
                                  className={`font-semibold ${getScoreColor(evaluation.scores.lintScore?.score ?? 0)}`}
                                >
                                  {(
                                    evaluation.scores.lintScore?.score ?? 0
                                  ).toFixed(3)}
                                </span>
                              </div>
                            </TableCell>
                            <TableCell>
                              <div className="flex items-center gap-2">
                                <span
                                  className={`font-semibold ${getScoreColor(evaluation.scores.testScore?.score ?? 0)}`}
                                >
                                  {(
                                    evaluation.scores.testScore?.score ?? 0
                                  ).toFixed(3)}
                                </span>
                              </div>
                            </TableCell>
                            <TableCell>
                              <div className="flex items-center gap-2">
                                <span>
                                  {evaluation.metrics.duration.metric.toFixed(
                                    2
                                  )}
                                </span>
                                {comparisonData && (
                                  <span
                                    className={getDiffColor(
                                      comparisonData.durationDiff,
                                      true
                                    )}
                                  >
                                    ({formatDiff(comparisonData.durationDiff)})
                                  </span>
                                )}
                              </div>
                            </TableCell>
                            <TableCell>
                              <div className="flex items-center gap-2">
                                <span>
                                  {evaluation.metrics.totalTokens.metric.toLocaleString()}
                                </span>
                                {comparisonData && (
                                  <span
                                    className={getDiffColor(
                                      comparisonData.tokenDiff.toString(),
                                      true
                                    )}
                                  >
                                    (
                                    {formatDiff(
                                      comparisonData.tokenDiff.toLocaleString()
                                    )}
                                    )
                                  </span>
                                )}
                              </div>
                            </TableCell>
                          </TableRow>
                        );
                      })}
                  </>
                );
              })}
            </TableBody>
          </Table>
        </div>

        <div className="text-sm text-muted-foreground text-center">
          Showing {filteredSummaries.length} models with{" "}
          {filteredSummaries.reduce((sum, s) => sum + s.totalEvaluations, 0)}{" "}
          total evaluations
          {globalComparison && (
            <span className="ml-2">
              • Comparing all models to {globalComparison}
            </span>
          )}
        </div>
      </div>
    </TooltipProvider>
  );
}
