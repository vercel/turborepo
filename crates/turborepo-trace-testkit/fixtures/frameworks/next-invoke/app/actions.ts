'use server';

import { actionClient } from '../lib/safe-action';

export async function createTeam(formData: FormData) {
  return { ok: true, name: formData.get('name') };
}

export async function renameTeam(id: string, formData: FormData) {
  return { ok: true, id, name: formData.get('name') };
}

export async function deleteTeam(id: string) {
  return { ok: true, id };
}

// Safe-action style: the export is the action.
export const inviteMember = actionClient
  .schema({ email: 'string' })
  .action(async ({ parsedInput }) => {
    return { invited: parsedInput.email };
  });
