import { useQuery } from '@tanstack/react-query';
import { loadSession } from '@/api/server';
import { SESSION_STALE_MS } from '@/config';

/** Сессия серверного режима (пользователь или гость); общий кэш для шлюза, меню и админки. */
export function useSession() {
  return useQuery({ queryKey: ['session'], queryFn: loadSession, retry: false, staleTime: SESSION_STALE_MS });
}
