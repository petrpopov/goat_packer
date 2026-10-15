import { randomId } from '../utils/randomId';
import { useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import { detectProfile } from '@/api/services';

/** File objects stringify to {}, so each selection needs its own key. Abort obsolete uploads. */
export function useProfileDetection(file: File | null) {
  const selection = useMemo(() => (file ? randomId() : null), [file]);
  return useQuery({
    queryKey: ['profile-detection', selection],
    enabled: file !== null,
    queryFn: ({ signal }) => {
      if (!file) throw new Error('Файл не выбран');
      return detectProfile(file, signal);
    },
    retry: false,
    staleTime: Infinity,
    gcTime: 0,
  });
}
