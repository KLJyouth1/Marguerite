export function DiffView({ diff }: { diff: string }): JSX.Element {
  const lines = diff.length > 0 ? diff.split('\n') : [];
  return (
    <div className="diff" role="region" aria-label="Unified diff">
      {lines.length === 0 ? (
        <p className="diff__empty">No textual changes were produced.</p>
      ) : (
        lines.map((line, i) => {
          const kind = line.startsWith('+++') || line.startsWith('---')
            ? 'meta'
            : line.startsWith('@@')
              ? 'hunk'
              : line.startsWith('+')
                ? 'add'
                : line.startsWith('-')
                  ? 'del'
                  : 'ctx';
          return (
            <div key={i} className={`diff__line diff__line--${kind}`}>
              <span className="diff__gutter" aria-hidden="true">
                {line.charAt(0) || ' '}
              </span>
              <span className="diff__text">{line.slice(1)}</span>
            </div>
          );
        })
      )}
    </div>
  );
}
