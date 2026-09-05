import type { Metadata } from "next";

import { WorkspaceClient } from "../../workspace-client";

interface WorkspacePageProps {
  readonly params: Promise<{ readonly id: string }>;
}

export async function generateMetadata({
  params
}: WorkspacePageProps): Promise<Metadata> {
  const { id } = await params;
  return { title: `Workspace ${id}` };
}

export default async function WorkspacePage({ params }: WorkspacePageProps) {
  const { id } = await params;
  return <WorkspaceClient workspaceId={id} />;
}
