'use client';

import { useActionState, useTransition } from 'react';
import { useAction } from 'next-safe-action/hooks';
import { renameTeam, inviteMember } from '../app/actions';
import { createTeam, deleteTeam } from '../lib/actions';

export function TeamForm({ id }: { id: string }) {
  const [state, formAction] = useActionState(createTeam, null);
  const rename = renameTeam.bind(null, id);
  const [pending, startTransition] = useTransition();
  const invite = useAction(inviteMember);

  return (
    <div>
      <form action={formAction}>
        <input name="name" />
      </form>
      <form action={rename}>
        <input name="name" />
      </form>
      <button
        disabled={pending}
        onClick={() => startTransition(async () => { await deleteTeam(id); })}
      >
        delete
      </button>
      <button onClick={() => invite.execute({ email: 'a@example.com' })}>invite</button>
      {String(state)}
    </div>
  );
}
