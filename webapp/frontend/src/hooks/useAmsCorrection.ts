import { useState } from 'react';
import type { PrinterProfile } from '@/api/types';

/**
 * Состояние «Коррекция времени смены прутка (AMS)». Длительность смены по умолчанию берётся из
 * профиля и подменяется при смене принтера; правка пользователя живёт только для того профиля,
 * в котором сделана. Без значения в профиле (тулчейнджер) коррекция недоступна.
 */
export function useAmsCorrection(profile: PrinterProfile | null) {
  const [on, setOn] = useState(false);
  const [edit, setEdit] = useState<{ profileId: string; value: string } | null>(null);
  const profileSeconds = profile?.filamentChangeSeconds ?? null;
  const available = profile === null || profileSeconds !== null;
  const seconds =
    profile !== null && edit?.profileId === profile.id
      ? edit.value
      : profileSeconds === null
        ? ''
        : String(profileSeconds);
  return {
    available,
    on: on && available,
    setOn,
    seconds,
    setSeconds: (value: string) => setEdit(profile ? { profileId: profile.id, value } : null),
  };
}
