export type SupportedOS = "darwin" | "linux" | "windows";
export type SupportedArch = "x64" | "arm64";
export type HumanArch = "64" | "arm64";
export type NpmOs = "darwin" | "linux" | "win32";

export interface Platform {
  os: SupportedOS;
  arch: SupportedArch;
}
