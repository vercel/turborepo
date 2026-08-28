const AI_GATEWAY_MODELS_URL = "https://ai-gateway.vercel.sh/v1/models";

export interface GatewayModel {
  readonly id: string;
  readonly name: string;
  readonly ownedBy: string;
}

interface ModelCandidate {
  readonly id?: unknown;
  readonly name?: unknown;
  readonly owned_by?: unknown;
  readonly supported_parameters?: unknown;
  readonly type?: unknown;
}

/** Keep only language models that can call the factory agent's tools. */
export function parseGatewayModels(value: unknown): readonly GatewayModel[] {
  if (typeof value !== "object" || value === null) return [];
  const data = (value as { readonly data?: unknown }).data;
  if (!Array.isArray(data)) return [];

  return data
    .flatMap((candidate: ModelCandidate) => {
      if (
        typeof candidate.id !== "string" ||
        typeof candidate.name !== "string" ||
        typeof candidate.owned_by !== "string" ||
        candidate.type !== "language" ||
        !Array.isArray(candidate.supported_parameters) ||
        !candidate.supported_parameters.includes("tools")
      ) {
        return [];
      }
      return [
        {
          id: candidate.id,
          name: candidate.name,
          ownedBy: candidate.owned_by
        }
      ];
    })
    .sort((left, right) => left.name.localeCompare(right.name));
}

export async function fetchGatewayModels(): Promise<readonly GatewayModel[]> {
  const response = await fetch(AI_GATEWAY_MODELS_URL, {
    headers: { accept: "application/json" },
    next: { revalidate: 3600 }
  });
  if (!response.ok) {
    throw new Error(`AI Gateway models request failed (${response.status}).`);
  }
  return parseGatewayModels(await response.json());
}
