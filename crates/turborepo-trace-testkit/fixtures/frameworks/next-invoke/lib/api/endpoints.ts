export const API_TEAMS = '/api/teams';
export const API_ITEMS_VERSION = (version: string): string =>
  `/api/${version}/items`;
export const API_ROUTES = {
  HEALTH: '/api/health',
  JOBS: '/api/jobs',
} as const;
