export const TEAMS_PATH = '/api/teams';
export const itemsPath = (version: string): string => `/api/${version}/items`;
export const ROUTES = {
  health: '/api/health',
  jobs: '/api/jobs',
} as const;
