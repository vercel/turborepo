export const blogPath = (slug: string) => `/blog/${slug}`;

export async function gotoPricing(page) {
  await page.goto('/pricing');
}
