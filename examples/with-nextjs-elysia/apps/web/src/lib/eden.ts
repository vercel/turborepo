import { treaty } from '@elysiajs/eden'
import { app } from '@/app/api/v1/[[...slugs]]/route'


export const api =
  // process is defined on server side and build time
  typeof process !== 'undefined'
    ? treaty<typeof app>(app).api
    : treaty<typeof app>('localhost:3001').api