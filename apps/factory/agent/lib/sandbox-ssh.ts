export function sandboxSshCommand(name: string): string {
  return `sandbox ssh ${name}`;
}

const SSHABLE_STATUSES = new Set(["running", "stopped"]);

export function isSandboxSSHable(status: string): boolean {
  return SSHABLE_STATUSES.has(status);
}
