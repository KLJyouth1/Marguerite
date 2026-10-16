import { useCallback, useRef, useState } from 'react';

export interface AsyncState<T> {
  data: T | null;
  loading: boolean;
  error: string | null;
  /** Monotonic id; bump on each successful run so views can key animations. */
  runId: number;
}

/**
 * Standardized fetch lifecycle: keeps loading / error / data in one place so
 * every panel renders honest empty / loading / error states from the same shape.
 * Never swallows the error — it is surfaced to the caller via the thrown value.
 */
export function useAsync<T>() {
  const [state, setState] = useState<AsyncState<T>>({
    data: null,
    loading: false,
    error: null,
    runId: 0,
  });
  const runIdRef = useRef(0);

  const run = useCallback(async (fn: () => Promise<T>, opts?: { reset?: boolean }): Promise<T> => {
    if (opts?.reset ?? true) {
      setState((prev) => ({ ...prev, loading: true, error: null }));
    } else {
      setState((prev) => ({ ...prev, loading: true }));
    }
    try {
      const data = await fn();
      runIdRef.current += 1;
      setState({ data, loading: false, error: null, runId: runIdRef.current });
      return data;
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Unexpected error';
      setState((prev) => ({ ...prev, loading: false, error: message }));
      throw err;
    }
  }, []);

  return { ...state, run };
}
