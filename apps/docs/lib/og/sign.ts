import { createHmac } from "node:crypto";

const OG_SECRET = process.env.OG_IMAGE_SECRET || "fallback-secret-for-dev";

/**
 * Normalizes parameters into a consistent string for signing.
 */
function normalizeParams(params: Record<string, string>): string {
  const sortedKeys = Object.keys(params).sort();
  return sortedKeys.map((key) => `${key}=${params[key]}`).join("&");
}

/**
 * Creates a signature for OG image URL parameters (Node.js runtime).
 * This prevents unauthorized generation of OG images with arbitrary content.
 */
export function signOgParams(params: Record<string, string>): string {
  const data = normalizeParams(params);
  return createHmac("sha256", OG_SECRET)
    .update(data)
    .digest("hex")
    .slice(0, 16);
}

/**
 * Verifies a signature for OG image URL parameters (Node.js runtime).
 * Returns true if the signature is valid.
 */
export function verifyOgSignature(
  params: Record<string, string>,
  signature: string
): boolean {
  const expectedSignature = signOgParams(params);
  return signature === expectedSignature;
}

/**
 * Creates a signed OG image URL for docs pages.
 */
export function createSignedDocsOgUrl(
  slugSegments: string[],
  basePath?: string
): string {
  const path = slugSegments.join("/");
  const sig = signOgParams({ path });

  const base = basePath ? `${basePath}/og` : "/og";
  return `${base}/${path}?sig=${sig}`;
}

/**
 * Creates a signed OG image URL for blog pages.
 */
export function createSignedBlogOgUrl(version: string): string {
  const sig = signOgParams({ version });
  return `/api/og/blog?version=${encodeURIComponent(version)}&sig=${sig}`;
}

/**
 * Creates a signed OG image URL for general pages (home, showcase, etc.).
 * If title is empty, generates an OG image with just the logo.
 */
export function createSignedOgUrl(title: string): string {
  const sig = signOgParams({ title });
  const params = new URLSearchParams({ title, sig });
  return `/api/og?${params.toString()}`;
}
