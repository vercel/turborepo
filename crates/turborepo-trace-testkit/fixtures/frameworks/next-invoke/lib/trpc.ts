import { createTRPCReact } from '@trpc/react-query';
import type { AppRouter } from '../server/root';

export const api = createTRPCReact<AppRouter>();
