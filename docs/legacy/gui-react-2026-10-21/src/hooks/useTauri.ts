import { useEffect, useState } from "react"

export const useTauri = () => {
  // Detect Tauri environment; graceful fallback for browser dev server.
  const [available, setAvailable] = useState<boolean | null>(null);
  useEffect(() => {
    try {
      const w = window as unknown as { __TAURI_INTERNALS__?: unknown };
      setAvailable(!!w.__TAURI_INTERNALS__);
    } catch {
      setAvailable(false);
    }
  }, []);
  return available === true;
};

