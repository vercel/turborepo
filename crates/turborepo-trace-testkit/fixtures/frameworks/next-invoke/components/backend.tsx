'use client';

import { apiFetch } from '../lib/api/fetch-api';

export function Backend() {
  async function load() {
    // Served by the backend behind the same origin, not by a route of this app.
    const projects = await apiFetch('/v9/projects');
    const status = await fetch('https://status.example.com/api/v2/status.json');
    return [projects, status];
  }
  return <button onClick={load}>load</button>;
}
