import { useState } from 'react';
import type { ResearchResult } from '../types/contract';
import { api } from '../api/client';
import { useAsync } from '../hooks/useAsync';
import { useWorkbenchStore } from '../store/workbenchStore';
import { Button } from '../components/ui/Button';
import { Icon } from '../components/ui/icons';
import { EmptyState, ErrorState, LoadingState } from '../components/ui/States';

export function ResearchPanel(): JSX.Element {
  const [q, setQ] = useState('');
  const researchState = useAsync<ResearchResult>();
  const pushToast = useWorkbenchStore((s) => s.pushToast);

  const runResearch = async () => {
    if (!q.trim()) return;
    try {
      const res = await researchState.run(() => api.research(q.trim()));
      pushToast('success', `Researched "${res.query}"`);
    } catch (err) {
      pushToast('error', err instanceof Error ? err.message : 'Research failed');
    }
  };

  return (
    <div className="feature">
      <p className="feature__lede">
        Online research (联网). Attempts a real network call when a provider key is configured; otherwise falls back to the local knowledge base.
      </p>

      <div className="filecard__form--inline">
        <input
          className="input input--grow"
          placeholder="Ask the web…"
          value={q}
          onChange={(e) => setQ(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') void runResearch();
          }}
          aria-label="Research query"
        />
        <Button variant="primary" icon="globe" onClick={runResearch} disabled={researchState.loading || !q.trim()} aria-label="Run research">
          Research
        </Button>
      </div>

      {researchState.loading ? <LoadingState label="Researching the web…" /> : null}
      {researchState.error ? <ErrorState title="Research failed" message={researchState.error} /> : null}

      {researchState.data ? (
        <div className="research">
          {researchState.data.synthesized ? (
            <section className="research__synthesis" aria-label="Synthesized answer">
              <h3 className="research__title">Synthesis</h3>
              <p className="research__text">{researchState.data.synthesized}</p>
            </section>
          ) : null}

          <section aria-label="Sources">
            <h3 className="research__title">Sources</h3>
            {researchState.data.results.length === 0 ? (
              <p className="filecard__muted">No sources returned (likely local fallback).</p>
            ) : (
              <ul className="research__list">
                {researchState.data.results.map((r, i) => (
                  <li key={i} className="research__item">
                    <a className="research__link" href={r.url} target="_blank" rel="noopener noreferrer">
                      <Icon name="globe" size={14} />
                      {r.title}
                    </a>
                    <p className="research__snippet">{r.snippet}</p>
                  </li>
                ))}
              </ul>
            )}
          </section>
        </div>
      ) : (
        <EmptyState icon="globe" title="No research yet" hint="Run a query to synthesize results from the web or the local fallback." />
      )}
    </div>
  );
}
