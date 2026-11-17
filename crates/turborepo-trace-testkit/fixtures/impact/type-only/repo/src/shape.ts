import type { Shape } from './types';
export const square = (s: number): Shape => ({ area: () => s * s });
