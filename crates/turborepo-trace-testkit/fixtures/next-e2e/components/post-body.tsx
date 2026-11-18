export function PostBody({ slug }: { slug: string }) {
  return <article data-slug={slug}>A post about {slug}.</article>;
}
