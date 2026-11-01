import { useEffect, useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import { fetchProfiles } from '@/api/services';
import { pushToast } from '@/store/useToastStore';
import { apiErrorMessage } from '@/utils/jobs';

/**
 * Каталог принтеров (один запрос на все страницы — общий ключ кэша). notifyErrors — показать
 * ошибку загрузки тостом; страницы смонтированы одновременно, поэтому тост просит одна-две, не все.
 */
export function useProfiles(notifyErrors = false) {
  const query = useQuery({ queryKey: ['profiles'], queryFn: fetchProfiles });
  const profiles = useMemo(() => query.data ?? [], [query.data]);
  useEffect(() => {
    if (notifyErrors && query.isError) pushToast(apiErrorMessage(query.error, 'Не удалось загрузить профили'), 'error');
  }, [notifyErrors, query.isError, query.error]);
  return { profiles, loading: query.isPending };
}
