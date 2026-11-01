import { useEffect, useState } from 'react';
import { CLOCK_TICK_MS } from '@/config';

/** Текущее время, тикающее раз в CLOCK_TICK_MS, только пока active (секундомер этапа между опросами). */
export function useNow(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    const timer = window.setInterval(() => setNow(Date.now()), CLOCK_TICK_MS);
    return () => window.clearInterval(timer);
  }, [active]);
  return now;
}
