import { names } from './names';

export const icon = (name: string) => `<svg data-icon="${name}" data-known="${names.includes(name)}"/>`;
