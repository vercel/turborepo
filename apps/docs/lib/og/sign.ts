import { createHmac, timingSafeEqual } from "node:crypto";

const HMAC_SHA256_HEX_LENGTH = 64;
const HEX_SIGNATURE_PATTERN = /^[0-9a-f]+$/i;

function getOgSecret(): string | null {
  return process.env.OG_IMAGE_SECRET || null;
}

function requireOgSecret(): string {
  const secret = getOgSecret();

  if (!secret) {
    throw new Error("OG_IMAGE_SECRET is not configured");
  }

  return secret;
}

/**
 * Normalizes parameters into a consistent string for signing.
 */
function normalizeParams(params: Record<string, string>): string {
  const searchParams = new URLSearchParams();

  for (const key of Object.keys(params).sort()) {
    searchParams.set(key, params[key]);
  }

  return searchParams.toString();
}

function createSignature(
  params: Record<string, string>,
  secret: string
): string {
  const data = normalizeParams(params);

  return createHmac("sha256", secret).update(data).digest("hex");
}

/**
 * Creates a signature for OG image URL parameters (Node.js runtime).
 * This prevents unauthorized generation of OG images with arbitrary content.
 */
export function signOgParams(params: Record<string, string>): string {
  return createSignature(params, requireOgSecret());
}

/**
 * Verifies a signature for OG image URL parameters (Node.js runtime).
 * Returns true if the signature is valid.
 */
export function verifyOgSignature(
  params: Record<string, string>,
  signature: string
): boolean {
  const secret = getOgSecret();

  if (!secret) {
    console.error("OG_IMAGE_SECRET is not configured");
    return false;
  }

  if (
    signature.length !== HMAC_SHA256_HEX_LENGTH ||
    !HEX_SIGNATURE_PATTERN.test(signature)
  ) {
    return false;
  }

  const expectedSignature = createSignature(params, secret);

  if (signature.length !== expectedSignature.length) {
    return false;
  }

  const signatureBuffer = Buffer.from(signature, "hex");
  const expectedSignatureBuffer = Buffer.from(expectedSignature, "hex");

  if (signatureBuffer.length !== expectedSignatureBuffer.length) {
    return false;
  }

  return timingSafeEqual(signatureBuffer, expectedSignatureBuffer);
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
