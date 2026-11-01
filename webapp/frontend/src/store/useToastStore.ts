import { create } from 'zustand';
import { TOAST_TTL_MS } from '@/config';

export type ToastKind = 'success' | 'error';

export interface Toast {
  id: number;
  kind: ToastKind;
  message: string;
}

let nextToastId = 1;

interface ToastState {
  toasts: Toast[];
  pushToast: (message: string, kind?: ToastKind) => void;
  dismissToast: (id: number) => void;
}

export const useToastStore = create<ToastState>((set, get) => ({
  toasts: [],
  pushToast: (message, kind = 'success') => {
    const id = nextToastId++;
    set((state) => ({ toasts: [...state.toasts, { id, kind, message }] }));
    setTimeout(() => get().dismissToast(id), TOAST_TTL_MS);
  },
  dismissToast: (id) => set((state) => ({ toasts: state.toasts.filter((toast) => toast.id !== id) })),
}));

export function pushToast(message: string, kind: ToastKind = 'success'): void {
  useToastStore.getState().pushToast(message, kind);
}
