// Served as it is, without a locale: `embed` is a fixed first segment.
export default async function EmbeddedPoll({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  return <iframe title={`Poll ${id}`} />;
}
