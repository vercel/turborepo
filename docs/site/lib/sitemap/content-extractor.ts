import { createHash } from "node:crypto";
import { parseHTML } from "linkedom";
import type { ContentExtractionOptions } from "./types";
import { DEFAULT_CONTENT_OPTIONS } from "./types";

/**
 * Extracts meaningful content from HTML and generates a hash.
 *
 * Only considers changes that warrant a lastmod update:
 * - Text changes within the main content area
 * - Image changes (src attributes)
 *
 * Ignores:
 * - Whitespace changes
 * - Number changes (e.g., view counts, timestamps)
 * - Changes outside the main content area (nav, footer, etc.)
 */
export function extractContentHash(
  html: string,
  options: ContentExtractionOptions = DEFAULT_CONTENT_OPTIONS
): string {
  const { document } = parseHTML(html);

  // Find the main content area
  const mainContent = findMainContent(document, options.mainContentSelectors);

  // Use main content if found, otherwise fallback to body
  const contentElement = mainContent ?? document.body;
  return extractAndHashContent(contentElement, options);
}

/**
 * Find the main content element using selectors in order of preference
 */
function findMainContent(
  document: Document,
  selectors: Array<string>
): Element | null {
  for (const selector of selectors) {
    const element = document.querySelector(selector);
    if (element) {
      return element;
    }
  }
  return null;
}

/**
 * Extract content from an element and generate a hash
 */
function extractAndHashContent(
  element: Element,
  options: ContentExtractionOptions
): string {
  const contentParts: Array<string> = [];

  // Extract text content
  const textContent = extractTextContent(element, options);
  contentParts.push(textContent);

  // Extract image sources if enabled
  if (options.includeImages) {
    const imageSources = extractImageSources(element);
    contentParts.push(imageSources.join("|"));
  }

  return hashString(contentParts.join("\n"));
}

/**
 * Extract and normalize text content from an element
 */
function extractTextContent(
  element: Element,
  options: ContentExtractionOptions
): string {
  // Get raw text content
  let text = element.textContent || "";

  // Normalize whitespace if enabled
  if (options.normalizeWhitespace) {
    text = normalizeWhitespace(text);
  }

  // Strip numbers if enabled (to ignore view counts, timestamps, etc.)
  if (options.stripNumbers) {
    text = stripNumbers(text);
  }

  return text;
}

/**
 * Extract image sources from an element
 */
function extractImageSources(element: Element): Array<string> {
  const images = element.querySelectorAll("img");
  const sources: Array<string> = [];

  for (const img of images) {
    const src = img.getAttribute("src");
    if (src) {
      // Normalize the src (remove query params that might change)
      const normalizedSrc = normalizeImageSrc(src);
      sources.push(normalizedSrc);
    }
  }

  return sources.sort(); // Sort for consistent hashing
}

/**
 * Normalize whitespace in text
 */
function normalizeWhitespace(text: string): string {
  return text
    .replace(/\s+/g, " ") // Collapse multiple whitespace to single space
    .trim();
}

/**
 * Strip numbers from text to ignore dynamic counts
 */
function stripNumbers(text: string): string {
  return (
    text
      // Remove standalone numbers (not part of version strings like v1.2.3)
      .replace(/(?<!\d\.)\b\d+\b(?!\.\d)/g, "")
      // Remove common dynamic patterns (times like 10:30:00 AM)
      .replace(/\d{1,2}:\d{2}(?::\d{2})?\s*(?:AM|PM|am|pm)?/g, "")
      // Remove dates like 01/15/2024
      .replace(/\d{1,2}\/\d{1,2}\/\d{2,4}/g, "")
      // Remove ISO dates
      .replace(/\d{4}-\d{2}-\d{2}/g, "")
      // Clean up resulting multiple spaces
      .replace(/\s+/g, " ")
      .trim()
  );
}

/**
 * Normalize image src by removing cache-busting params
 */
function normalizeImageSrc(src: string): string {
  try {
    // Handle relative URLs
    if (!src.startsWith("http")) {
      // Just remove query params for relative URLs
      return src.split("?")[0];
    }

    const url = new URL(src);
    // Remove common cache-busting params
    url.searchParams.delete("v");
    url.searchParams.delete("_");
    url.searchParams.delete("t");
    url.searchParams.delete("cb");
    // Return origin + pathname to preserve the image source while removing query params
    return url.origin + url.pathname;
  } catch {
    return src.split("?")[0];
  }
}

/**
 * Generate SHA-256 hash of a string
 */
function hashString(content: string): string {
  return createHash("sha256").update(content).digest("hex");
}
