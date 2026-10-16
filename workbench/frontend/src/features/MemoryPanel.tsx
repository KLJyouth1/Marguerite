import { useEffect, useState } from 'react';
import type { MemoryItem, MemoryLayerInfo } from '../types/contract';
import { api } from '../api/client';
import { useAsync } from '../hooks/useAsync';
import { useWorkbenchStore } from '../store/workbenchStore';
import { Button } from '../components/ui/Button';
import { EmptyState, ErrorState, LoadingState } from '../components/ui/States';

const LAYER_DESC: Record<string, string> = {
  L1: 'Sensory — raw input buffers (transient, not persisted)',
  L2: 'Short-term — active session context',
  L3: 'Working — task-scoped structured scratchpad',
  L4: 'Episodic — session/event logs, time-ordered',
  L5: 'Semantic — facts + knowledge base',
  L6: 'Procedural — skills / playbooks / distilled experience',
  L7: 'Meta — self-model: profile, capabilities, reflection',
};

function occupancy(count: number, capacity: number): number {
  if (capacity <= 0) return 0;
  return Math.min(100, Math.round((count / capacity) * 100));
}

function LayerCard({ layer }: { layer: MemoryLayerInfo }): JSX.Element {
  const [detail, setDetail] = useState<'none' | 'query' | 'store'>('none');
  const [q, setQ] = useState('');
  const [storeKind, setStoreKind] = useState('note');
  const [storeContent, setStoreContent] = useState('');
  const [storeRefs, setStoreRefs] = useState('');

  const queryState = useAsync<MemoryItem[]>();
  const storeState = useAsync<MemoryItem>();
  const pushToast = useWorkbenchStore((s) => s.pushToast);

  const runQuery = async () => {
    if (!q.trim()) return;
    try {
      await queryState.run(() => api.queryMemory(layer.layer, q.trim()));
    } catch (err) {
      pushToast('error', err instanceof Error ? err.message : 'Query failed');
    }
  };

  const runStore = async () => {
    if (!storeContent.trim()) return;
    try {
      await storeState.run(() =>
        api.storeMemory(layer.layer, {
          layer: layer.layer,
          kind: storeKind.trim() || 'note',
          content: storeContent.trim(),
          refs: storeRefs
            .split(',')
            .map((r) => r.trim())
            .filter(Boolean),
        }),
      );
      pushToast('success', `Stored item in ${layer.layer}`);
      setStoreContent('');
      setStoreRefs('');
    } catch (err) {
      pushToast('error', err instanceof Error ? err.message : 'Store failed');
    }
  };

  const pct = occupancy(layer.count, layer.capacity);

  return (
    <article className="layercard">
      <header className="layercard__head">
        <div className="layercard__id">
          <span className="layercard__badge">{layer.layer}</span>
          <div>
            <h3 className="layercard__name">{layer.name}</h3>
            <p className="layercard__desc">{LAYER_DESC[layer.layer] ?? layer.name}</p>
          </div>
        </div>
        <div className="layercard__counts">
          <span className="layercard__count">{layer.count}</span>
          <span className="layercard__cap">/ {layer.capacity}</span>
        </div>
      </header>

      <div
        className="layercard__bar"
        role="progressbar"
        aria-valuenow={pct}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-label={`${layer.layer} occupancy`}
      >
        <span className="layercard__bar-fill" style={{ width: `${pct}%` }} />
      </div>

      <div className="layercard__actions">
        <Button variant={detail === 'query' ? 'primary' : 'secondary'} size="sm" icon="search" onClick={() => setDetail((d) => (d === 'query' ? 'none' : 'query'))}>
          Query
        </Button>
        <Button variant={detail === 'store' ? 'primary' : 'secondary'} size="sm" icon="plus" onClick={() => setDetail((d) => (d === 'store' ? 'none' : 'store'))}>
          Store
        </Button>
      </div>

      {detail === 'query' ? (
        <div className="layercard__panel">
          <div className="filecard__form--inline">
            <input
              className="input"
              placeholder={`Search ${layer.layer}…`}
              value={q}
              onChange={(e) => setQ(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') void runQuery();
              }}
              aria-label="Memory query"
            />
            <Button variant="primary" size="sm" icon="search" onClick={runQuery} disabled={queryState.loading || !q.trim()} aria-label="Run query">
              Run
            </Button>
          </div>
          {queryState.loading ? <LoadingState label="Querying…" /> : null}
          {queryState.error ? <ErrorState title="Query failed" message={queryState.error} /> : null}
          {queryState.data && queryState.data.length === 0 ? (
            <p className="filecard__muted">No items matched.</p>
          ) : null}
          {queryState.data && queryState.data.length > 0 ? (
            <ul className="memlist">
              {queryState.data.map((item) => (
                <li key={item.id} className="memitem">
                  <span className="chip chip--mono">{item.kind}</span>
                  <p className="memitem__content">{item.content}</p>
                  {item.refs.length > 0 ? <p className="memitem__refs">refs: {item.refs.join(', ')}</p> : null}
                </li>
              ))}
            </ul>
          ) : null}
        </div>
      ) : null}

      {detail === 'store' ? (
        <div className="layercard__panel">
          <div className="filecard__form">
            <div className="filecard__form--inline">
              <input
                className="input"
                placeholder="kind (e.g. fact, note, skill)"
                value={storeKind}
                onChange={(e) => setStoreKind(e.target.value)}
                aria-label="Item kind"
              />
              <input
                className="input input--grow"
                placeholder="refs, comma-separated (optional)"
                value={storeRefs}
                onChange={(e) => setStoreRefs(e.target.value)}
                aria-label="References"
              />
            </div>
            <textarea
              className="input input--area"
              placeholder="Content to store…"
              value={storeContent}
              onChange={(e) => setStoreContent(e.target.value)}
              rows={3}
              aria-label="Item content"
            />
            <div className="filecard__form-actions">
              <Button variant="primary" size="sm" icon="plus" onClick={runStore} disabled={storeState.loading || !storeContent.trim()}>
                {storeState.loading ? 'Storing…' : 'Store item'}
              </Button>
            </div>
          </div>
          {storeState.error ? <ErrorState title="Store failed" message={storeState.error} /> : null}
          {storeState.data ? <p className="filecard__muted">Stored as {storeState.data.id}.</p> : null}
        </div>
      ) : null}
    </article>
  );
}

export function MemoryPanel(): JSX.Element {
  const layersState = useAsync<MemoryLayerInfo[]>();

  useEffect(() => {
    void layersState.run(() => api.getMemoryLayers());
  }, [layersState]);

  return (
    <div className="feature">
      <p className="feature__lede">
        Seven-layer memory model (L1 sensory → L7 meta). Occupancy reflects stored vs. capacity; query and store against any layer.
      </p>
      {layersState.loading && !layersState.data ? <LoadingState label="Loading memory layers…" /> : null}
      {layersState.error && !layersState.data ? (
        <ErrorState title="Could not load memory layers" message={layersState.error} onRetry={() => void layersState.run(() => api.getMemoryLayers())} />
      ) : null}
      {layersState.data && layersState.data.length === 0 ? (
        <EmptyState icon="layers" title="No memory layers reported" hint="The backend returned an empty layer list." />
      ) : null}
      {layersState.data && layersState.data.length > 0 ? (
        <div className="layer-grid">
          {layersState.data.map((layer) => (
            <LayerCard key={layer.layer} layer={layer} />
          ))}
        </div>
      ) : null}
    </div>
  );
}
