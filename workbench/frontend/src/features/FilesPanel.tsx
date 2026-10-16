import { useState } from 'react';
import type { FileCard as FileCardModel } from '../types/contract';
import { api } from '../api/client';
import { useAsync } from '../hooks/useAsync';
import { useWorkbenchStore } from '../store/workbenchStore';
import { Button } from '../components/ui/Button';
import { EmptyState, ErrorState, LoadingState } from '../components/ui/States';
import { FileCard } from '../components/FileCard';

export function FilesPanel(): JSX.Element {
  const [cards, setCards] = useState<FileCardModel[]>([]);
  const [path, setPath] = useState('');
  const loadState = useAsync<FileCardModel>();
  const pushToast = useWorkbenchStore((s) => s.pushToast);

  const handleLoad = async () => {
    const trimmed = path.trim();
    if (!trimmed) return;
    try {
      const card = await loadState.run(() => api.loadFile(trimmed));
      setCards((prev) => [card, ...prev.filter((c) => c.file_id !== card.file_id)]);
      pushToast('success', `Loaded ${card.path}`);
      setPath('');
    } catch (err) {
      pushToast('error', err instanceof Error ? err.message : 'Failed to load file');
    }
  };

  const handleUpdate = (updated: FileCardModel) => {
    setCards((prev) => prev.map((c) => (c.file_id === updated.file_id ? updated : c)));
  };

  const handleRemove = (fileId: string) => {
    setCards((prev) => prev.filter((c) => c.file_id !== fileId));
  };

  return (
    <div className="feature">
      <div className="loaderow">
        <input
          className="input input--grow"
          placeholder="Absolute or workspace-relative path, e.g. src/main.rs"
          value={path}
          onChange={(e) => setPath(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') void handleLoad();
          }}
          aria-label="File path to load"
        />
        <Button variant="primary" icon="file" onClick={handleLoad} disabled={loadState.loading || !path.trim()}>
          {loadState.loading ? 'Loading…' : 'Load file'}
        </Button>
      </div>

      {loadState.error && cards.length === 0 ? (
        <ErrorState title="Could not load file" message={loadState.error} onRetry={handleLoad} />
      ) : null}

      {cards.length === 0 && !loadState.loading ? (
        <EmptyState
          icon="file"
          title="No files loaded"
          hint="Load a source file by path to inspect its structure, modify it, ask questions, or chat with it."
          action={
            <div className="samplepaths">
              <span className="samplepaths__label">Try:</span>
              {['src/main.rs', 'README.md', 'package.json'].map((p) => (
                <button key={p} type="button" className="chip chip--button" onClick={() => setPath(p)}>
                  {p}
                </button>
              ))}
            </div>
          }
        />
      ) : null}

      {loadState.loading && cards.length === 0 ? <LoadingState label="Loading file…" /> : null}

      <div className="filecard-list">
        {cards.map((card) => (
          <FileCard key={card.file_id} card={card} onUpdate={handleUpdate} onRemove={handleRemove} />
        ))}
      </div>
    </div>
  );
}
