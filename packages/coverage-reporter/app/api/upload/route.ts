import { NextRequest, NextResponse } from "next/server";
import { parseCoverageReport } from "@/lib/coverage";
import { storeCoverageReport, getBranchSummary } from "@/lib/blob";
import { calculateCoverageDiff } from "@/lib/coverage";
import type { LlvmCovReport, UploadResponse } from "@/lib/types";

/**
 * POST /api/upload
 *
 * Upload coverage data from CI.
 * Expects JSON body with: { sha, branch, report }
 * Or query params: ?sha=xxx&branch=yyy with report as body
 */
export async function POST(request: NextRequest) {
  // Verify API token
  const authHeader = request.headers.get("authorization");
  const expectedToken = process.env.COVERAGE_API_TOKEN;

  if (!expectedToken) {
    return NextResponse.json(
      { error: "Server configuration error: COVERAGE_API_TOKEN not set" },
      { status: 500 }
    );
  }

  if (authHeader !== `Bearer ${expectedToken}`) {
    return NextResponse.json({ error: "Unauthorized" }, { status: 401 });
  }

  try {
    const contentType = request.headers.get("content-type") || "";

    let sha: string;
    let branch: string;
    let report: LlvmCovReport;

    if (contentType.includes("application/json")) {
      // JSON body with all fields
      const body = await request.json();

      // Support both nested and flat formats
      if (body.report) {
        // Nested: { sha, branch, report }
        sha = body.sha;
        branch = body.branch;
        report = body.report;
      } else if (body.data && body.type === "llvm.coverage.json.export") {
        // Flat: report is the body, sha/branch from query params
        sha = request.nextUrl.searchParams.get("sha") || "";
        branch = request.nextUrl.searchParams.get("branch") || "";
        report = body;
      } else {
        return NextResponse.json(
          {
            error:
              "Invalid request format. Expected { sha, branch, report } or llvm-cov JSON with query params."
          },
          { status: 400 }
        );
      }
    } else {
      return NextResponse.json(
        { error: "Content-Type must be application/json" },
        { status: 400 }
      );
    }

    // Validate required fields
    if (!sha || !branch) {
      return NextResponse.json(
        { error: "Missing required fields: sha and branch" },
        { status: 400 }
      );
    }

    if (!report || !report.data || !Array.isArray(report.data)) {
      return NextResponse.json(
        { error: "Invalid coverage report format" },
        { status: 400 }
      );
    }

    // Parse and transform the report
    const coverageReport = parseCoverageReport(report, sha, branch);

    // Store in blob
    const { url } = await storeCoverageReport(coverageReport);

    // Get baseline for diff (main/master branch)
    let diff = null;
    if (branch !== "main" && branch !== "master") {
      const mainSummary = await getBranchSummary("main");
      const masterSummary = await getBranchSummary("master");
      const baseline = mainSummary || masterSummary;

      if (baseline) {
        diff = calculateCoverageDiff(coverageReport.summary, baseline);
      }
    }

    const response: UploadResponse & { diff?: typeof diff } = {
      success: true,
      sha,
      summary: coverageReport.summary,
      url,
      diff
    };

    return NextResponse.json(response);
  } catch (error) {
    console.error("Upload error:", error);
    return NextResponse.json(
      {
        error: error instanceof Error ? error.message : "Internal server error"
      },
      { status: 500 }
    );
  }
}

/**
 * GET /api/upload
 *
 * Health check endpoint
 */
export async function GET() {
  return NextResponse.json({
    status: "ok",
    message: "Coverage upload endpoint ready"
  });
}
