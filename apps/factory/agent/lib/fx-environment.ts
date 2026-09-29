export function fxEnvironment(oidcToken: string): Record<string, string> {
  return {
    FX_AUTO_UPGRADE: "0",
    FX_PERMISSION_MODE: "yolo",
    VERCEL_OIDC_TOKEN: oidcToken
  };
}
