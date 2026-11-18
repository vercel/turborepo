import NextLink from 'next/link';
import type { ComponentProps } from 'react';

// The app's own link: `next/link` without prefetching. It hands its props on, so the `href`
// of a link is written where the component is used.
export function Link({ prefetch = false, ...props }: ComponentProps<typeof NextLink>) {
  return <NextLink prefetch={prefetch} {...props} />;
}
