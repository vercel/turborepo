import { chromium, type Browser } from "playwright";
import fs from "fs/promises";
import path from "path";
import pixelmatch from "pixelmatch";
import { PNG } from "pngjs";

export interface VisualDiffOptions {
  url: string;
  outputDir: string;
  evalPath: string;
  enabled?: boolean;
  threshold?: number; // Pixelmatch threshold (0-1), default 0.1 (higher = more tolerant)
}

export interface VisualDiffResult {
  success: boolean;
  screenshotPath?: string;
  expectedPath?: string;
  diffPath?: string;
  pixelDifference?: number;
  percentDifference?: number;
  error?: string;
}

/**
 * Capture a screenshot of the running app and compare with expected using pixelmatch
 */
export async function captureAndCompare(
  options: VisualDiffOptions
): Promise<VisualDiffResult> {
  if (!options.enabled) {
    return { success: true };
  }

  let browser: Browser | undefined;

  try {
    // Launch browser
    browser = await chromium.launch({ headless: true });
    const page = await browser.newPage();

    // Navigate to the app
    await page.goto(options.url, { waitUntil: "networkidle" });

    // Wait a bit for any client-side hydration
    await page.waitForTimeout(1000);

    // Capture screenshot
    const screenshotDir = path.join(options.outputDir, "screenshots");
    await fs.mkdir(screenshotDir, { recursive: true });

    const screenshotPath = path.join(screenshotDir, "homepage.png");
    await page.screenshot({ path: screenshotPath, fullPage: true });

    // Check if expected screenshot exists
    const evalsDir = path.join(process.cwd(), "evals");
    const expectedDir = path.join(
      evalsDir,
      options.evalPath,
      "expected",
      "screenshots"
    );
    const expectedPath = path.join(expectedDir, "homepage.png");

    const expectedExists = await fs
      .stat(expectedPath)
      .then(() => true)
      .catch(() => false);

    if (!expectedExists) {
      // No expected screenshot to compare against
      return {
        success: true,
        screenshotPath,
        expectedPath: undefined
      };
    }

    // Load images using pngjs
    const expectedBuffer = await fs.readFile(expectedPath);
    const actualBuffer = await fs.readFile(screenshotPath);

    const expectedPng = PNG.sync.read(expectedBuffer);
    const actualPng = PNG.sync.read(actualBuffer);

    // Check if dimensions match
    if (
      expectedPng.width !== actualPng.width ||
      expectedPng.height !== actualPng.height
    ) {
      return {
        success: false,
        screenshotPath,
        expectedPath,
        error: `Image dimensions don't match. Expected: ${expectedPng.width}x${expectedPng.height}, Actual: ${actualPng.width}x${actualPng.height}`
      };
    }

    // Create diff image
    const { width, height } = expectedPng;
    const diffPng = new PNG({ width, height });

    // Use pixelmatch to compare
    const threshold = options.threshold ?? 0.1; // Default to 0.1 (fairly tolerant)
    const numDiffPixels = pixelmatch(
      expectedPng.data,
      actualPng.data,
      diffPng.data,
      width,
      height,
      { threshold }
    );

    const totalPixels = width * height;
    const percentDifference = (numDiffPixels / totalPixels) * 100;

    // Save diff image if there are differences
    let diffPath: string | undefined;
    if (numDiffPixels > 0) {
      diffPath = path.join(screenshotDir, "diff.png");
      await fs.writeFile(diffPath, PNG.sync.write(diffPng));
    }

    // Consider success if less than 1% of pixels differ
    const maxAllowedPercent = 1.0; // 1%
    const success = percentDifference < maxAllowedPercent;

    return {
      success,
      screenshotPath,
      expectedPath,
      diffPath,
      pixelDifference: numDiffPixels,
      percentDifference
    };
  } catch (error) {
    return {
      success: false,
      error: error instanceof Error ? error.message : String(error)
    };
  } finally {
    if (browser) {
      await browser.close();
    }
  }
}
