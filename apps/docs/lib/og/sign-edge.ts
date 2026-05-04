const HMAC_SHA256_HEX_LENGTH = 64;

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

/**
 * Converts an ArrayBuffer to a hex string.
 */
function bufferToHex(buffer: ArrayBuffer): string {
  return Array.from(new Uint8Array(buffer))
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

function hexToArrayBuffer(hex: string): ArrayBuffer | null {
  if (hex.length % 2 !== 0 || !/^[0-9a-f]*$/i.test(hex)) {
    return null;
  }

  const bytes = new Uint8Array(hex.length / 2);
  for (let index = 0; index < bytes.length; index++) {
    bytes[index] = Number.parseInt(hex.slice(index * 2, index * 2 + 2), 16);
  }

  return bytes.buffer;
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
  const secret = requireOgSecret();

  const key = await crypto.subtle.importKey(
    "raw",
    encoder.encode(secret),
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
  const secret = getOgSecret();

  if (!secret) {
    console.error("OG_IMAGE_SECRET is not configured");
    return false;
  }

  if (signature.length !== HMAC_SHA256_HEX_LENGTH) {
    return false;
  }

  const signatureBytes = hexToArrayBuffer(signature);
  if (!signatureBytes) {
    return false;
  }

  const data = normalizeParams(params);
  const encoder = new TextEncoder();
  const key = await crypto.subtle.importKey(
    "raw",
    encoder.encode(secret),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["verify"]
  );

  return crypto.subtle.verify(
    "HMAC",
    key,
    signatureBytes,
    encoder.encode(data)
  );
}
