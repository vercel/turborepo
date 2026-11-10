'use client';

import { saveSettings } from '../app/actions';
import { Button } from './button';

export function SettingsForm() {
  return (
    <form action={saveSettings}>
      <input name="name" />
      <Button>Save</Button>
    </form>
  );
}
