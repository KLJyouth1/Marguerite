import { create } from 'zustand';
import type { PanelId } from '../registry';

export type ToastKind = 'info' | 'success' | 'error';

export interface Toast {
  id: string;
  kind: ToastKind;
  message: string;
}

interface WorkbenchState {
  activePanel: PanelId;
  setActivePanel: (panel: PanelId) => void;

  toasts: Toast[];
  pushToast: (kind: ToastKind, message: string) => void;
  dismissToast: (id: string) => void;

  theme: 'light' | 'dark';
  toggleTheme: () => void;
}

function makeId(): string {
  if (typeof crypto !== 'undefined' && 'randomUUID' in crypto) {
    return crypto.randomUUID();
  }
  return `t_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 8)}`;
}

function applyTheme(theme: 'light' | 'dark'): void {
  if (typeof document !== 'undefined') {
    document.documentElement.setAttribute('data-theme', theme);
  }
}

export const useWorkbenchStore = create<WorkbenchState>((set, get) => ({
  activePanel: 'overview',
  setActivePanel: (panel) => set({ activePanel: panel }),

  toasts: [],
  pushToast: (kind, message) => {
    const id = makeId();
    set((state) => ({ toasts: [...state.toasts, { id, kind, message }] }));
    if (typeof window !== 'undefined') {
      window.setTimeout(() => get().dismissToast(id), 4200);
    }
  },
  dismissToast: (id) =>
    set((state) => ({ toasts: state.toasts.filter((toast) => toast.id !== id) })),

  theme: 'light',
  toggleTheme: () => {
    const next: 'light' | 'dark' = get().theme === 'light' ? 'dark' : 'light';
    applyTheme(next);
    set({ theme: next });
  },
}));
