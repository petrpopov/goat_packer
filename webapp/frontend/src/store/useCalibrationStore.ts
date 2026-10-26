import { create } from 'zustand';
import { readStoredChangeSeconds, storeChangeSeconds } from '@/utils/calibration';

interface CalibrationState {
  /** Коррекции, сохранённые кнопкой «Использовать как коррекцию»: { profileId: секунды }. */
  changeSeconds: Record<string, number>;
  saveChangeSeconds: (profileId: string, seconds: number) => void;
}

/**
 * Сохранённая длительность смены прутка по принтерам. Живёт в localStorage (без аккаунтов и
 * сервера), а стор нужен, чтобы уже открытые формы упаковки/расписания сразу подхватили значение.
 */
export const useCalibrationStore = create<CalibrationState>((set, get) => ({
  changeSeconds: readStoredChangeSeconds(),
  saveChangeSeconds: (profileId, seconds) => {
    storeChangeSeconds(profileId, seconds);
    set({ changeSeconds: { ...get().changeSeconds, [profileId]: seconds } });
  },
}));
