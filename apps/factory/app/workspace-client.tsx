"use client";

import dynamic from "next/dynamic";

const WorkspaceTerminal = dynamic(
  () =>
    import("./workspace-terminal").then((module) => module.WorkspaceTerminal),
  { ssr: false }
);

interface WorkspaceClientProps {
  readonly workspaceId: string;
}

export function WorkspaceClient({ workspaceId }: WorkspaceClientProps) {
  return <WorkspaceTerminal workspaceId={workspaceId} />;
}
