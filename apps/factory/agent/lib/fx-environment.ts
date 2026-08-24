import { getVercelOidcToken } from "@vercel/oidc";

import type { WorkspaceModelOption } from "./workspace";

const FX_MODELS_URL = "https://ai-gateway.vercel.sh/coding-agent/v1/models";

export function fxEnvironment(
  oidcToken: string,
  model?: string
): Record<string, string> {
  return {
    FX_AUTO_UPGRADE: "0",
    FX_PERMISSION_MODE: "yolo",
    ...(model ? { FX_MODEL: model } : {}),
    VERCEL_OIDC_TOKEN: oidcToken
  };
}

export function parseFxModelCatalog(
  value: unknown
): readonly WorkspaceModelOption[] {
  if (!isObject(value) || !Array.isArray(value.data)) return [];
  const seen = new Set<string>();
  return value.data.flatMap((entry) => {
    if (!isObject(entry) || !isFxModelId(entry.id) || seen.has(entry.id))
      return [];
    seen.add(entry.id);
    return [
      {
        id: entry.id,
        name:
          typeof entry.name === "string" && entry.name.trim()
            ? entry.name.trim()
            : entry.id
      }
    ];
  });
}

function isFxModelId(value: unknown): value is string {
  return (
    typeof value === "string" &&
    value.length <= 200 &&
    /^[A-Za-z0-9][A-Za-z0-9._-]*\/[A-Za-z0-9][A-Za-z0-9._:-]*$/.test(value)
  );
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export async function listFxModels(options?: {
  readonly fetch?: typeof fetch;
  readonly oidcToken?: string | null;
}): Promise<readonly WorkspaceModelOption[]> {
  let oidcToken = options?.oidcToken;
  if (oidcToken === undefined) {
    try {
      oidcToken = await getVercelOidcToken();
    } catch {
      oidcToken = null;
    }
  }
  const request = options?.fetch ?? fetch;
  const init: RequestInit & { readonly next: { readonly revalidate: number } } =
    {
      headers: {
        accept: "application/json",
        ...(oidcToken ? { authorization: `Bearer ${oidcToken}` } : {})
      },
      next: { revalidate: 300 }
    };
  let response = await request(FX_MODELS_URL, init);
  if (!response.ok && oidcToken) {
    response = await request(FX_MODELS_URL, {
      headers: { accept: "application/json" },
      next: { revalidate: 300 }
    } as RequestInit);
  }
  if (!response.ok)
    throw new Error(`AI Gateway model catalog returned ${response.status}.`);
  const models = parseFxModelCatalog(await response.json());
  if (models.length === 0)
    throw new Error("AI Gateway model catalog did not contain any fx models.");
  return models;
}
