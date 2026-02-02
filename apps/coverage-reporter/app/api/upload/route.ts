import { NextRequest, NextResponse } from "next/server";
import { parseCoverageReport } from "@/lib/coverage";
import { storeCoverageReport, getBranchSummary } from "@/lib/blob";
import { calculateCoverageDiff } from "@/lib/coverage";
import type { LlvmCovReport, UploadResponse } from "@/lib/types";

// Route segment config for large payloads
export const maxDuration = 60;

/**
 * Stream and parse JSON from request body in chunks.
 * Avoids V8 string length limits by not buffering entire body as a string.
 */
async function streamParseJson(request: NextRequest): Promise<unknown> {
  const reader = request.body?.getReader();
  if (!reader) {
    throw new Error("No request body");
  }

  const chunks: Uint8Array[] = [];
  let totalLength = 0;

  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    chunks.push(value);
    totalLength += value.length;
  }

  // Combine chunks into a single buffer and parse
  const combined = new Uint8Array(totalLength);
  let offset = 0;
  for (const chunk of chunks) {
    combined.set(chunk, offset);
    offset += chunk.length;
  }

  const text = new TextDecoder().decode(combined);
  return JSON.parse(text);
}

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
      // JSON body with all fields - use streaming parser for large payloads
      const body = (await streamParseJson(request)) as Record<string, unknown>;

      // Support both nested and flat formats
      if (body.report) {
        // Nested: { sha, branch, report }
        sha = body.sha as string;
        branch = body.branch as string;
        report = body.report as LlvmCovReport;
      } else if (body.data && body.type === "llvm.coverage.json.export") {
        // Flat: report is the body, sha/branch from query params
        sha = request.nextUrl.searchParams.get("sha") || "";
        branch = request.nextUrl.searchParams.get("branch") || "";
        report = body as unknown as LlvmCovReport;
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
