import { getJson } from '../../lib/api';

export default async function Team() {
  await getJson('/api/health');
  // No such handler: not `/[team]/settings` with team = "api".
  await fetch('/api/settings');
  return null;
}
