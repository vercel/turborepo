export default function sortDate(a, b) {
  if (!a.frontMatter || !a.frontMatter.date) return -1;
  if (!b.frontMatter || !b.frontMatter.date) return -1;
  return new Date(a.frontMatter.date) > new Date(b.frontMatter.date) ? -1 : 1;
}
