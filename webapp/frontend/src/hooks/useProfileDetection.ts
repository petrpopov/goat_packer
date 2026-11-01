import { randomId } from '../utils/randomId';
import { useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { detectProfile } from '@/api/services';

/** Сколько файла ушло на сервер, байт; null — отправка ещё не началась или уже закончилась. */
export type UploadState = { sent: number; total: number } | null;

/**
 * File objects stringify to {}, so each selection needs its own key. Abort obsolete uploads.
 * Прогресс отправки — для полосы под полем файла (на сервере файл весит десятки МБ).
 */
export function useProfileDetection(file: File | null) {
  const selection = useMemo(() => (file ? randomId() : null), [file]);
  const [upload, setUpload] = useState<UploadState>(null);
  const query = useQuery({
    queryKey: ['profile-detection', selection],
    enabled: file !== null,
    queryFn: async ({ signal }) => {
      if (!file) throw new Error('Файл не выбран');
      setUpload({ sent: 0, total: file.size });
      try {
        return await detectProfile(file, signal, (sent, total) => setUpload({ sent, total }));
      } finally {
        setUpload(null);
      }
    },
    retry: false,
    staleTime: Infinity,
    gcTime: 0,
  });
  return { ...query, upload };
}
