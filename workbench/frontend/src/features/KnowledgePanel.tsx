import { useState } from 'react';
import type { KBChunk } from '../types/contract';
import { api } from '../api/client';
import { useAsync } from '../hooks/useAsync';
import { useWorkbenchStore } from '../store/workbenchStore';
import { Button } from '../components/ui/Button';
import { EmptyState, ErrorState, LoadingState } from '../components/ui/States';

export function KnowledgePanel(): JSX.Element {
  const [q, setQ] = useState('');
  const [k, setK] = useState(5);
  const searchState = useAsync<KBChunk[]>();

  const [ingestText, setIngestText] = useState('');
  const [ingestPath, setIngestPath] = useState('');
  const [docId, setDocId] = useState('');
  const ingestState = useAsync<{ indexed_chunks: number }>();
  const pushToast = useWorkbenchStore((s) => s.pushToast);

  const runSearch = async () => {
    if (!q.trim()) return;
    try {
      await searchState.run(() => api.searchKB(q.trim(), k));
    } catch (err) {
      pushToast('error', err instanceof Error ? err.message : 'Search failed');
    }
  };

  const runIngest = async () => {
    if (!ingestText.trim() && !ingestPath.trim()) {
      pushToast('error', 'Provide text or a file path to ingest');
      return;
    }
    try {
      const res = await ingestState.run(() =>
        api.ingestKB({
          text: ingestText.trim() || null,
          path: ingestPath.trim() || null,
          doc_id: docId.trim() || `doc_${Date.now().toString(36)}`,
        }),
      );
      pushToast('success', `Indexed ${res.indexed_chunks} chunk(s)`);
      setIngestText('');
      setIngestPath('');
      setDocId('');
    } catch (err) {
      pushToast('error', err instanceof Error ? err.message : 'Ingest failed');
    }
  };

  return (
    <div className="feature">
      <p className="feature__lede">Search the semantic knowledge base (L5) and ingest new text or files.</p>

      <section className="kb-block" aria-label="Knowledge base search">
        <h3 className="kb-block__title">Search</h3>
        <div className="filecard__form--inline">
          <input
            className="input input--grow"
            placeholder="Search the knowledge base…"
            value={q}
            onChange={(e) => setQ(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') void runSearch();
            }}
            aria-label="Search query"
          />
          <label className="kb-k">
            <span>k</span>
            <input
              className="input input--narrow"
              type="number"
              min={1}
              max={20}
              value={k}
              onChange={(e) => setK(Math.max(1, Number(e.target.value) || 1))}
              aria-label="Number of results"
            />
          </label>
          <Button variant="primary" icon="search" onClick={runSearch} disabled={searchState.loading || !q.trim()} aria-label="Run search">
            Search
          </Button>
        </div>
        {searchState.loading ? <LoadingState label="Searching…" /> : null}
        {searchState.error ? <ErrorState title="Search failed" message={searchState.error} /> : null}
        {searchState.data && searchState.data.length === 0 ? (
          <p className="filecard__muted">No chunks matched.</p>
        ) : null}
        {searchState.data && searchState.data.length > 0 ? (
          <ul className="kblist">
            {searchState.data.map((chunk, i) => (
              <li key={i} className="kbchunk">
                <div className="kbchunk__meta">
                  <span className="chip chip--mono">{chunk.source}</span>
                  <span className="kbchunk__score">score {chunk.score.toFixed(3)}</span>
                </div>
                <p className="kbchunk__text">{chunk.text}</p>
              </li>
            ))}
          </ul>
        ) : null}
      </section>

      <section className="kb-block" aria-label="Ingest into knowledge base">
        <h3 className="kb-block__title">Ingest</h3>
        <div className="filecard__form">
          <textarea
            className="input input--area"
            placeholder="Paste text to index…"
            value={ingestText}
            onChange={(e) => setIngestText(e.target.value)}
            rows={4}
            aria-label="Text to ingest"
          />
          <div className="filecard__form--inline">
            <input
              className="input"
              placeholder="Or file path (optional)"
              value={ingestPath}
              onChange={(e) => setIngestPath(e.target.value)}
              aria-label="File path to ingest"
            />
            <input
              className="input"
              placeholder="doc_id"
              value={docId}
              onChange={(e) => setDocId(e.target.value)}
              aria-label="Document id"
            />
          </div>
          <div className="filecard__form-actions">
            <Button variant="primary" icon="plus" onClick={runIngest} disabled={ingestState.loading}>
              {ingestState.loading ? 'Indexing…' : 'Ingest'}
            </Button>
          </div>
        </div>
        {ingestState.error ? <ErrorState title="Ingest failed" message={ingestState.error} /> : null}
        {ingestState.data ? (
          <p className="filecard__muted">Indexed {ingestState.data.indexed_chunks} chunk(s).</p>
        ) : null}
      </section>

      {!searchState.data && !ingestState.data && !searchState.loading ? (
        <EmptyState icon="search" title="Knowledge base is idle" hint="Run a search or ingest content to populate results here." />
      ) : null}
    </div>
  );
}
