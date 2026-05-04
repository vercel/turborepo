type HeaderGetter = {
  get(name: string): string | null;
};

export function getClientIp(headers: HeaderGetter): string {
  const forwardedFor = headers.get("x-forwarded-for")?.split(",") ?? [];
  const proxyAppendedIp = forwardedFor
    .map((ip) => ip.trim())
    .filter(Boolean)
    .at(-1);

  return proxyAppendedIp || "unknown";
}
