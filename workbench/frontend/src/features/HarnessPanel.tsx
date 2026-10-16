import { useEffect, useState } from 'react';
import type { ExpertRole, HarnessAdapter, HarnessResult } from '../types/contract';
import { api } from '../api/client';
import { useAsync } from '../hooks/useAsync';
import { useWorkbenchStore } from '../store/workbenchStore';
import { Button } from '../components/ui/Button';
import { Icon } from '../components/ui/icons';
import { EmptyState, ErrorState, LoadingState } from '../components/ui/States';

export function HarnessPanel(): JSX.Element {
  const adaptersState = useAsync<HarnessAdapter[]>();
  const [selected, setSelected] = useState<string | null>(null);
  const [task, setTask] = useState('');
  const dispatchState = useAsync<HarnessResult>();

  // Distilled-expert roster: the same five roles the Expert console exposes,
  // surfaced here as the brain trust the harness can consult.
  const expertsState = useAsync<ExpertRole[]>();

  const pushToast = useWorkbenchStore((s) => s.pushToast);
  const setActivePanel = useWorkbenchStore((s) => s.setActivePanel);

  useEffect(() => {
    void adaptersState.run(() => api.getHarnessAdapters());
  }, [adaptersState]);

  useEffect(() => {
    void expertsState.run(() => api.getExperts());
  }, [expertsState]);

  const handleDispatch = async () => {
    if (!task.trim()) return;
    try {
      const res = await dispatchState.run(() => api.dispatchHarness(task.trim(), selected));
      pushToast(res.status === 'ok' ? 'success' : 'info', `Dispatch via ${res.adapter} — ${res.status}`);
    } catch (err) {
      pushToast('error', err instanceof Error ? err.message : 'Dispatch failed');
    }
  };

  return (
    <div className="feature">
      <p className="feature__lede">
        Codex-class harness bridge. Adapters are checked for availability on the host; dispatch runs the task and
        normalizes the result. The distilled-expert roster below is the brain trust the harness consults.
      </p>

      {/* ---------- Capability list: harness adapters ---------- */}
      {adaptersState.loading && !adaptersState.data ? <LoadingState label="Loading adapters…" /> : null}
      {adaptersState.error && !adaptersState.data ? (
        <ErrorState
          title="Could not load adapters"
          message={adaptersState.error}
          onRetry={() => void adaptersState.run(() => api.getHarnessAdapters())}
        />
      ) : null}
      {adaptersState.data && adaptersState.data.length === 0 ? (
        <EmptyState icon="cpu" title="No harness adapters" hint="The backend returned an empty adapter list." />
      ) : null}

      {adaptersState.data && adaptersState.data.length > 0 ? (
        <div className="adapter-grid">
          {adaptersState.data.map((adapter) => (
            <article
              key={adapter.id}
              className={`adapter-card ${selected === adapter.id ? 'adapter-card--active' : ''}`}
            >
              <header className="adapter-card__head">
                <span className="adapter-card__glyph" aria-hidden="true">
                  <Icon name="cpu" size={18} />
                </span>
                <div>
                  <h3 className="adapter-card__name">{adapter.name}</h3>
                  <span className="chip chip--mono">{adapter.id}</span>
                </div>
                <span
                  className={`status-dot status-dot--${adapter.available ? 'on' : 'off'}`}
                  aria-label={adapter.available ? 'Available' : 'Unavailable'}
                >
                  {adapter.available ? 'available' : 'unavailable'}
                </span>
              </header>
              {adapter.command ? <code className="adapter-card__cmd">{adapter.command}</code> : null}
              <ul className="adapter-card__practices">
                {adapter.best_practices.map((p, i) => (
                  <li key={i}>{p}</li>
                ))}
              </ul>
              <Button
                variant={selected === adapter.id ? 'primary' : 'secondary'}
                size="sm"
                onClick={() => setSelected(adapter.id)}
                aria-label={`Select ${adapter.name}`}
              >
                {selected === adapter.id ? 'Selected' : 'Use'}
              </Button>
            </article>
          ))}
          <article className="adapter-card adapter-card--internal">
            <header className="adapter-card__head">
              <span className="adapter-card__glyph" aria-hidden="true">
                <Icon name="cpu" size={18} />
              </span>
              <div>
                <h3 className="adapter-card__name">Marguerite internal</h3>
                <span className="chip chip--mono">internal</span>
              </div>
              <span className="status-dot status-dot--on" aria-label="Available">
                always
              </span>
            </header>
            <p className="adapter-card__cmd">Plan → Act → Observe loop (no external CLI required)</p>
            <Button
              variant={selected === null ? 'primary' : 'secondary'}
              size="sm"
              onClick={() => setSelected(null)}
              aria-label="Use internal harness"
            >
              {selected === null ? 'Selected' : 'Use'}
            </Button>
          </article>
        </div>
      ) : null}

      {/* ---------- Distilled-expert roster ---------- */}
      <section className="kb-block" aria-label="Distilled expert roster">
        <h3 className="kb-block__title">Distilled expert roster</h3>
        <p className="feature__lede">
          Five distilled roles ({expertsState.data ? expertsState.data.length : '—'}). Each is a real callable
          analysis — pick one in the Experts console to run its template against a payload.
        </p>

        {expertsState.loading && !expertsState.data ? <LoadingState label="Loading expert roster…" /> : null}
        {expertsState.error && !expertsState.data ? (
          <ErrorState
            title="Could not load expert roster"
            message={expertsState.error}
            onRetry={() => void expertsState.run(() => api.getExperts())}
          />
        ) : null}
        {expertsState.data && expertsState.data.length === 0 ? (
          <EmptyState icon="brain" title="No experts available" hint="The backend returned an empty expert list." />
        ) : null}

        {expertsState.data && expertsState.data.length > 0 ? (
          <div className="expert-grid">
            {expertsState.data.map((role) => (
              <article key={role.role} className="expert-card">
                <header className="expert-card__head">
                  <span className="expert-card__glyph" aria-hidden="true">
                    <Icon name="brain" size={18} />
                  </span>
                  <div>
                    <h4 className="expert-card__title">{role.title}</h4>
                    <span className="chip chip--mono">{role.role}</span>
                  </div>
                </header>
                <div className="expert-card__section">
                  <span className="expert-card__label">Distilled from</span>
                  <div className="expert-card__tags">
                    {role.distilled_from.map((d) => (
                      <span key={d} className="chip">
                        {d}
                      </span>
                    ))}
                  </div>
                </div>
                <p className="expert-card__exp">
                  {role.skills.length} distilled skill{role.skills.length === 1 ? '' : 's'} ·{' '}
                  <button
                    type="button"
                    className="expert-card__link"
                    onClick={() => setActivePanel('experts')}
                    aria-label={`Open ${role.title} in the Experts console`}
                  >
                    Open console
                  </button>
                </p>
              </article>
            ))}
          </div>
        ) : null}
      </section>

      {/* ---------- Dispatch ---------- */}
      <section className="analyze-box" aria-label="Harness dispatch">
        <header className="analyze-box__head">
          <Icon name="send" size={18} />
          <div>
            <h3>Dispatch task</h3>
            <p className="analyze-box__hint">
              {selected ? `Adapter: ${selected}` : 'Adapter: internal (auto)'}
            </p>
          </div>
        </header>
        <div className="filecard__form--inline">
          <input
            className="input input--grow"
            placeholder="Describe the task to dispatch…"
            value={task}
            onChange={(e) => setTask(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') void handleDispatch();
            }}
            aria-label="Task to dispatch"
          />
          <Button
            variant="primary"
            icon="send"
            onClick={handleDispatch}
            disabled={dispatchState.loading || !task.trim()}
            aria-label="Dispatch"
          >
            Dispatch
          </Button>
        </div>

        {dispatchState.loading ? <LoadingState label="Dispatching…" /> : null}
        {dispatchState.error ? <ErrorState title="Dispatch failed" message={dispatchState.error} /> : null}
        {dispatchState.data ? (
          <div className="harness-result">
            <div className="harness-result__head">
              <span
                className={`status-dot status-dot--${
                  dispatchState.data.status === 'ok'
                    ? 'on'
                    : dispatchState.data.status === 'error'
                      ? 'off'
                      : 'warn'
                }`}
              >
                {dispatchState.data.status}
              </span>
              <span className="chip chip--mono">{dispatchState.data.adapter}</span>
              <span className="harness-result__dur">{dispatchState.data.duration_ms} ms</span>
            </div>
            {dispatchState.data.steps.length > 0 ? (
              <div className="harness-result__block">
                <span className="harness-result__label">Steps</span>
                <ol className="harness-result__steps">
                  {dispatchState.data.steps.map((s, i) => (
                    <li key={i}>{s}</li>
                  ))}
                </ol>
              </div>
            ) : null}
            {dispatchState.data.diffs.length > 0 ? (
              <div className="harness-result__block">
                <span className="harness-result__label">Diffs</span>
                {dispatchState.data.diffs.map((d, i) => (
                  <pre key={i} className="harness-result__diff">
                    {d}
                  </pre>
                ))}
              </div>
            ) : null}
          </div>
        ) : null}
      </section>
    </div>
  );
}
