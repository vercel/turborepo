import { createClient } from '@formium/client'

export const formium = createClient('5bcf69ce1726e8000149a091', {
  apiToken: process.env.FORMIUM_TOKEN,
})
