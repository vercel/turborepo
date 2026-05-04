const OG_SECRET = process.env.OG_IMAGE_SECRET || "fallback-secret-for-dev";
const HMAC_SHA256_HEX_LENGTH = 64;

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

function hexToUint8Array(hex: string): Uint8Array | null {
  if (hex.length % 2 !== 0 || !/^[0-9a-f]*$/i.test(hex)) {
    return null;
  }

  const bytes = new Uint8Array(hex.length / 2);
  for (let index = 0; index < bytes.length; index++) {
    bytes[index] = Number.parseInt(hex.slice(index * 2, index * 2 + 2), 16);
  }

  return bytes;
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

  return bufferToHex(signature);
}

/**
 * Verifies a signature for OG image URL parameters (Edge runtime).
 * Returns true if the signature is valid.
 */
export async function verifyOgSignatureEdge(
  params: Record<string, string>,
  signature: string
): Promise<boolean> {
  if (signature.length !== HMAC_SHA256_HEX_LENGTH) {
    return false;
  }

  const signatureBytes = hexToUint8Array(signature);
  if (!signatureBytes) {
    return false;
  }

  const data = normalizeParams(params);
  const encoder = new TextEncoder();
  const key = await crypto.subtle.importKey(
    "raw",
    encoder.encode(OG_SECRET),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["verify"]
  );

  return crypto.subtle.verify("HMAC", key, signatureBytes, encoder.encode(data));
}
