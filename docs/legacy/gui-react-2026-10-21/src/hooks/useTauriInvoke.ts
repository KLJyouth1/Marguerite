import { useCallback, useEffect, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTauri } from "./useTauri"

export function useTauriInvoke<T>(cmd: string, args?: Record<string, unknown>) {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const tauri = useTauri();

  const run = useCallback(
    async (extraArgs?: Record<string, unknown>) => {
      if (!tauri) {
        setError("非 Tauri 环境（浏览器预览）");
        return;
      }
      setLoading(true);
      setError(null);
      try {
        const result = await invoke<T>(cmd, { ...args, ...extraArgs });
        setData(result);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        setLoading(false);
      }
    },
    [tauri, cmd, JSON.stringify(args)]
  );

  useEffect(() => {
    run();
  }, [run]);

  return { data, error, loading, run, setData };
}

