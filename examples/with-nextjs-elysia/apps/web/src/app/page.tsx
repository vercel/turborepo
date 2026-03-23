// 'use client'

import { useQuery } from '@tanstack/react-query'
import { api } from '@/lib/eden'

// export default function Page() {
//   const { data: message, isLoading, error } = useQuery({
//     queryKey: ['hello'],
//     queryFn: () => api.get()
//   })

//   if (isLoading) return <div>Loading...</div>
//   if (error) return <div>Error: {error.message}</div>

//   return <h1>Hello, {message?.data ?? 'World'}</h1>
// }




export default async function Page() {
  const { data, error } = await api.get()


  if (error) return <div>Error: {String(error)}</div>

  return <h1>Hello, {data ?? 'World'}</h1>
}


