'use client';

import { api } from '../lib/trpc';

export function UserPanel({ id }: { id: string }) {
  const utils = api.useUtils();
  const user = api.user.byId.useQuery({ id });
  const posts = api.post.all.useSuspenseQuery();
  const create = api.user.create.useMutation({
    onSuccess: () => utils.user.list.invalidate(),
  });
  const comment = api.post.comments.add.useMutation();
  return (
    <button onClick={() => { create.mutate({ name: 'Bo' }); comment.mutate(); }}>
      {user.data?.name} {String(posts)}
    </button>
  );
}
