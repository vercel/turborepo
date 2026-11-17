import { path } from './paths';

export const icon = (name: string) => `<svg data-icon="${name}">${path(name)}</svg>`;
