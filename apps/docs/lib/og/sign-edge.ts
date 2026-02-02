const OG_SECRET = process.env.OG_IMAGE_SECRET || "fallback-secret-for-dev";

/**
 * Normalizes parameters into a consistent string for signing.
 */
function normalizeParams(params: Record<string, string>): string {
  const sortedKeys = Object.keys(params).sort();
  return sortedKeys.map((key) => `${key}=${params[key]}`).join("&");
}

/**
 * Converts an ArrayBuffer to a hex string.
 */
function bufferToHex(buffer: ArrayBuffer): string {
  return Array.from(new Uint8Array(buffer))
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

/**
 * Creates a signature for OG image URL parameters (Edge runtime).
 * Uses Web Crypto API which is available in edge environments.
 */
export async function signOgParamsEdge(
  params: Record<string, string>
): Promise<string> {
  const data = normalizeParams(params);
  const encoder = new TextEncoder();

  const key = await crypto.subtle.importKey(
    "raw",
    encoder.encode(OG_SECRET),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"]
  );

  const signature = await crypto.subtle.sign("HMAC", key, encoder.encode(data));

  return bufferToHex(signature).slice(0, 16);
}

/**
 * Verifies a signature for OG image URL parameters (Edge runtime).
 * Returns true if the signature is valid.
 */
export async function verifyOgSignatureEdge(
  params: Record<string, string>,
  signature: string
): Promise<boolean> {
  const expectedSignature = await signOgParamsEdge(params);
  return signature === expectedSignature;
}
