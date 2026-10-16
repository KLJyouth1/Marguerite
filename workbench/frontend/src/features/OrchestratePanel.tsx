import { useEffect, useState } from 'react';
import type { AgentRunResult, AgentToolInfo } from '../types/contract';
import { api } from '../api/client';
import { useAsync } from '../hooks/useAsync';
import { useWorkbenchStore } from '../store/workbenchStore';
import { Button } from '../components/ui/Button';
import { Icon } from '../components/ui/icons';
import { EmptyState, ErrorState, LoadingState } from '../components/ui/States';

/**
 * Orchestrate panel — drives the Codex-style agent loop (plan→act→observe→
 * repair) against the real backend and renders the step timeline it returns.
 * The sandboxed tool registry manifest is fetched once for transparency.
 */
export function OrchestratePanel(): JSX.Element {
  const [task, setTask] = useState('');
  const runState = useAsync<AgentRunResult>();
  const toolsState = useAsync<AgentToolInfo[]>();
  const pushToast = useWorkbenchStore((s) => s.pushToast);

  useEffect(() => {
    toolsState
      .run(async () => (await api.getAgentTools()).tools)
      .catch(() => undefined);
    // Mount-only manifest fetch.
  }, [toolsState]);

  const runTask = async () => {
    if (!task.trim()) return;
    try {
      const res = await runState.run(() => api.runAgent(task.trim()));
      pushToast(res.status === 'completed' ? 'success' : 'error',
        `run ${res.status} · ${res.steps.length} steps`);
    } catch (err) {
      pushToast('error', err instanceof Error ? err.message : 'agent run failed');
    }
  };

  return (
    <div className="feature">
      <p className="feature__lede">
        Codex-style agent loop: deterministic offline planner → sandboxed tools →
        observe → repair (bounded). Every step is appended to the session JSONL.
      </p>

      {toolsState.data ? (
        <div className="ov-toolbar" aria-label="Sandboxed tool registry">
          {toolsState.data.map((t) => (
            <span key={t.name} className="chip chip--mono" title={t.safe_resolve_guarded ? 'path guarded by safe_resolve' : t.required_args.join(', ') || 'no required args'}>
              {t.name}
            </span>
          ))}
        </div>
      ) : null}

      <div className="filecard__form--inline">
        <input
          className="input input--grow"
          placeholder='e.g. "read sample.py" or "search knowledge about bloom"'
          value={task}
          onChange={(e) => setTask(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') void runTask();
          }}
          aria-label="Agent task"
        />
        <Button variant="primary" icon="route" onClick={runTask} disabled={runState.loading || !task.trim()}>
          Run loop
        </Button>
      </div>

      {runState.loading ? <LoadingState label="Running the agent loop…" /> : null}
      {runState.error ? <ErrorState title="Run failed to start" message={runState.error} /> : null}

      {runState.data ? (
        <div className="feature">
          <div className="ov-toolbar">
            <span className={`status-dot ${runState.data.status === 'completed' ? 'status-dot--on' : 'status-dot--warn'}`}>
              {runState.data.status}
            </span>
            <span className="chip chip--mono">session {runState.data.session_id}</span>
            <span className="chip chip--mono">{runState.data.steps.length} steps</span>
          </div>

          <ol className="steps" aria-label="Step timeline">
            {runState.data.steps.map((st) => (
              <li key={st.index} className={`step ${st.ok ? 'step--ok' : 'step--fail'}`}>
                <span className="step__glyph"><Icon name={st.ok ? 'check' : 'alert'} size={14} /></span>
                <div className="step__body">
                  <header className="step__head">
                    <span className={`chip chip--mono ${st.is_repair ? 'chip--repair' : ''}`}>
                      {st.tool}
                      {st.is_repair ? ' · repair' : ''}
                    </span>
                    <span className="step__ms">{st.duration_ms} ms</span>
                  </header>
                  {!st.ok && st.error ? <code className="step__error">{st.error}</code> : null}
                  {st.ok ? <code className="step__detail">{summarizeDetail(st.detail)}</code> : null}
                </div>
              </li>
            ))}
          </ol>

          <section className="research__synthesis" aria-label="Agent answer">
            <h3 className="research__title">Answer</h3>
            <pre className="ov-answer">{runState.data.answer}</pre>
          </section>
        </div>
      ) : (
        <EmptyState
          icon="route"
          title="No run yet"
          hint="Describe a task; the planner routes it through the sandboxed tool registry."
        />
      )}
    </div>
  );
}

function summarizeDetail(detail: Record<string, unknown>): string {
  if ('entries' in detail) {
    const entries = (detail.entries as string[]) ?? [];
    return `listed ${String(detail.path)} — ${entries.slice(0, 8).join(', ')}`;
  }
  if ('preview' in detail) {
    return `read ${String(detail.path)} (${String(detail.bytes)}B): ${String(detail.preview).slice(0, 120)}`;
  }
  if ('hits' in detail) return `kb hits: ${JSON.stringify(detail.hits).slice(0, 140)}`;
  if ('items' in detail) return `${(detail.items as unknown[]).length} memory item(s)`;
  return JSON.stringify(detail).slice(0, 140);
}
