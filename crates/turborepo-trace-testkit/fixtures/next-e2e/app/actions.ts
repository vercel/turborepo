'use server';

import { formatName } from '../lib/format';

export async function saveSettings(data: FormData) {
  return { name: formatName(String(data.get('name') ?? '')) };
}
