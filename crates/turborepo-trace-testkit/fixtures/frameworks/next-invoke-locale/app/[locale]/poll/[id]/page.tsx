export default async function Poll({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  return <h1>Poll {id}</h1>;
}
