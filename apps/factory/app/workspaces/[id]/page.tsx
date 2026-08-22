import { WorkspaceClient } from "../../workspace-client";

interface WorkspacePageProps {
  readonly params: Promise<{ readonly id: string }>;
}

export default async function WorkspacePage({ params }: WorkspacePageProps) {
  const { id } = await params;
  return <WorkspaceClient workspaceId={id} />;
}
