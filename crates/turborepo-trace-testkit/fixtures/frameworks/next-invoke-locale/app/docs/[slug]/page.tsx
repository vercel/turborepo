export default async function Doc({ params }: { params: Promise<{ slug: string }> }) {
  const { slug } = await params;
  return <article>{slug}</article>;
}
