import { NextRequest, NextResponse } from "next/server";
import { getBranchSummary } from "@/lib/blob";

/**
 * GET /api/badge?branch=main
 *
 * Returns an SVG badge showing coverage percentage
 */
export async function GET(request: NextRequest) {
  const branch = request.nextUrl.searchParams.get("branch") || "main";
  const metric = request.nextUrl.searchParams.get("metric") || "lines";

  const summary = await getBranchSummary(branch);

  let percent = 0;
  let label = "coverage";

  if (summary) {
    switch (metric) {
      case "functions":
        percent = summary.functions.percent;
        label = "functions";
        break;
      case "branches":
        percent = summary.branches.percent;
        label = "branches";
        break;
      case "lines":
      default:
        percent = summary.lines.percent;
        label = "coverage";
    }
  }

  // Determine color based on percentage
  let color: string;
  if (percent >= 80) {
    color = "#4c1"; // green
  } else if (percent >= 60) {
    color = "#a3c51c"; // yellow-green
  } else if (percent >= 40) {
    color = "#dfb317"; // yellow
  } else if (percent >= 20) {
    color = "#fe7d37"; // orange
  } else {
    color = "#e05d44"; // red
  }

  const percentText = summary ? `${percent.toFixed(1)}%` : "N/A";

  // SVG badge (shields.io style)
  const svg = `
<svg xmlns="http://www.w3.org/2000/svg" width="110" height="20">
  <linearGradient id="b" x2="0" y2="100%">
    <stop offset="0" stop-color="#bbb" stop-opacity=".1"/>
    <stop offset="1" stop-opacity=".1"/>
  </linearGradient>
  <clipPath id="a">
    <rect width="110" height="20" rx="3" fill="#fff"/>
  </clipPath>
  <g clip-path="url(#a)">
    <path fill="#555" d="M0 0h63v20H0z"/>
    <path fill="${color}" d="M63 0h47v20H63z"/>
    <path fill="url(#b)" d="M0 0h110v20H0z"/>
  </g>
  <g fill="#fff" text-anchor="middle" font-family="DejaVu Sans,Verdana,Geneva,sans-serif" font-size="11">
    <text x="31.5" y="15" fill="#010101" fill-opacity=".3">${label}</text>
    <text x="31.5" y="14">${label}</text>
    <text x="85.5" y="15" fill="#010101" fill-opacity=".3">${percentText}</text>
    <text x="85.5" y="14">${percentText}</text>
  </g>
</svg>`.trim();

  return new NextResponse(svg, {
    headers: {
      "Content-Type": "image/svg+xml",
      "Cache-Control": "no-cache, no-store, must-revalidate"
    }
  });
}
