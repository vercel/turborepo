import { PostBody } from '@/components';

export default async function PostPage({ params }: { params: Promise<{ slug: string }> }) {
  const { slug } = await params;
  return (
    <main>
      <PostBody slug={slug} />
    </main>
  );
}
