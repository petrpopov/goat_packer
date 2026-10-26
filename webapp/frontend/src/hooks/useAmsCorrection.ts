import { useState } from 'react';
import type { PrinterProfile } from '@/api/types';
import { useCalibrationStore } from '@/store/useCalibrationStore';
import { defaultChangeSeconds } from '@/utils/calibration';

/**
 * Состояние «Коррекция времени смены прутка (AMS)». Длительность смены по умолчанию — сохранённый
 * результат калибровки этого принтера (страница «Калибровка AMS»), без него — значение профиля;
 * подменяется при смене принтера. Правка пользователя живёт только для того профиля, в котором
 * сделана. Без значения в профиле (тулчейнджер) коррекция недоступна.
 */
export function useAmsCorrection(profile: PrinterProfile | null) {
  const [on, setOn] = useState(false);
  const [edit, setEdit] = useState<{ profileId: string; value: string } | null>(null);
  const stored = useCalibrationStore((state) => (profile ? state.changeSeconds[profile.id] : undefined));
  const profileSeconds = profile?.filamentChangeSeconds ?? null;
  const available = profile === null || profileSeconds !== null;
  const seconds =
    profile !== null && edit?.profileId === profile.id ? edit.value : defaultChangeSeconds(profileSeconds, stored);
  return {
    available,
    on: on && available,
    setOn,
    seconds,
    setSeconds: (value: string) => setEdit(profile ? { profileId: profile.id, value } : null),
  };
}
