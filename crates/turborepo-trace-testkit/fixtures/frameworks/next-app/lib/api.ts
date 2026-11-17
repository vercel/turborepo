// Fetch wrappers: their URL parameter is forwarded, so their call sites are URL sites.
export const getJson = (url: string) => fetch(url).then((r) => r.json());

// Return types with object literals and arrows with return types.
export async function postJson(url: string, body: unknown): Promise<{ ok: boolean }> {
  return fetch(url, { method: 'POST', body: JSON.stringify(body) }).then((r) => r.json());
}

export const getText = async (url: string): Promise<string> => (await fetch(url)).text();
