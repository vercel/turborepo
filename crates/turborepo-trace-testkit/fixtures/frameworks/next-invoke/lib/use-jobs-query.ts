import { useQuery, useMutation } from '@tanstack/react-query';
import { apiFetch } from './api/fetch-api';

// A custom hook around the query: the invocation belongs to every component that uses it.
export function useJobsQuery(teamId: string) {
  return useQuery({
    queryKey: ['jobs', teamId],
    queryFn: () => apiFetch(`/jobs?teamId=${teamId}`),
  });
}

export function useHealth() {
  return useQuery({
    queryKey: ['health'],
    queryFn: async () => {
      const res = await fetch('/api/health');
      return res.json();
    },
  });
}

export function useDeleteItem() {
  return useMutation({
    mutationFn: (id: string) => fetch(`/api/v1/items/${id}`, { method: 'DELETE' }),
  });
}
