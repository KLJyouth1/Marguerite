import { useState } from 'react';
import type { ForwardResultDTO, TokenizeResult } from '../types/contract';
import { api } from '../api/client';
import { useAsync } from '../hooks/useAsync';
import { useWorkbenchStore } from '../store/workbenchStore';
import { Button } from '../components/ui/Button';
import { EmptyState, ErrorState, LoadingState } from '../components/ui/States';

/**
 * Model Lab panel — first-principles LLM math on REAL computed values:
 * byte-level BPE tokenisation (with merge trace) and a tiny GPT forward pass
 * whose per-head causal attention matrices are rendered as heatmaps.
 */
export function ModelLabPanel(): JSX.Element {
  const [text, setText] = useState('attention is all you need');
  const tokState = useAsync<TokenizeResult>();
  const fwdState = useAsync<ForwardResultDTO>();
  const pushToast = useWorkbenchStore((s) => s.pushToast);

  const runTokenize = async () => {
    if (!text.trim()) return;
    try {
      const res = await tokState.run(() => api.llmlabTokenize(text));
      pushToast('success', `${res.n_tokens} tokens · ${res.compression}x compression`);
    } catch (err) {
      pushToast('error', err instanceof Error ? err.message : 'tokenize failed');
    }
  };

  const runForward = async () => {
    if (!text.trim()) return;
    try {
      const res = await fwdState.run(() => api.llmlabForward(text.slice(0, 48)));
      pushToast('success', `loss ${res.loss?.toFixed(3)} · ${res.attention.length} layer(s)`);
    } catch (err) {
      pushToast('error', err instanceof Error ? err.message : 'forward failed');
    }
  };

  return (
    <div className="feature">
      <p className="feature__lede">
        First-principles lab: every number below is computed by the backend&apos;s
        dependency-free BPE tokenizer and TinyGPT forward pass — nothing is mocked.
      </p>

      <div className="filecard__form--inline">
        <input
          className="input input--grow"
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') void runTokenize();
          }}
          aria-label="Lab input text"
        />
        <Button variant="secondary" icon="spark" onClick={runTokenize} disabled={tokState.loading || !text.trim()}>
          Tokenize
        </Button>
        <Button variant="primary" icon="atom" onClick={runForward} disabled={fwdState.loading || !text.trim()}>
          Forward pass
        </Button>
      </div>

      {tokState.loading || fwdState.loading ? (
        <LoadingState label="Computing…" />
      ) : null}
      {tokState.error ? <ErrorState title="Tokenize failed" message={tokState.error} /> : null}
      {fwdState.error ? <ErrorState title="Forward failed" message={fwdState.error} /> : null}

      {tokState.data ? (
        <section className="panel" aria-label="Tokenization result">
          <header className="panel__head">
            <div><h3 className="panel__title">Byte-level BPE</h3></div>
            <span className={`status-dot ${tokState.data.decoded_roundtrip_ok ? 'status-dot--on' : 'status-dot--warn'}`}>
              roundtrip {tokState.data.decoded_roundtrip_ok ? 'ok' : 'lossy'}
            </span>
          </header>
          <div className="panel__body ov-tokstats">
            <span className="chip chip--mono">{tokState.data.n_tokens} tokens</span>
            <span className="chip chip--mono">vocab {tokState.data.vocab_size}</span>
            <span className="chip chip--mono">{tokState.data.compression}x compression</span>
            <span className="chip chip--mono">{tokState.data.trace.length} merges applied</span>
          </div>
        </section>
      ) : null}

      {fwdState.data ? (
        <section className="panel" aria-label="Forward pass telemetry">
          <header className="panel__head">
            <div>
              <h3 className="panel__title">Causal self-attention (layer 1)</h3>
              <p className="panel__subtitle">
                loss {fwdState.data.loss != null ? fwdState.data.loss.toFixed(4) : '—'} ·
                dim {fwdState.data.config.dim} · heads {fwdState.data.config.n_heads}
              </p>
            </div>
          </header>
          <div className="panel__body attn-wrap">
            {(fwdState.data.attention[0] ?? []).map((head, h) => (
              <figure key={h} className="attn" aria-label={`Head ${h + 1} attention matrix`}>
                <figcaption className="attn__cap">head {h + 1}</figcaption>
                <div
                  className="attn__grid"
                  style={{ gridTemplateColumns: `repeat(${head.length}, 1fr)` }}
                >
                  {head.flatMap((row, i) =>
                    // pad the lower triangle visually: row i has i+1 live keys
                    Array.from({ length: head.length }, (_, j) => {
                      const p = j < row.length ? row[j] : -1;
                      return (
                        <span
                          key={`${i}-${j}`}
                          className="attn__cell"
                          style={{
                            background: p < 0 ? 'var(--surface-2)' : `rgba(59,130,246,${Math.min(1, p).toFixed(3)})`,
                          }}
                          title={`q${i + 1}→k${j + 1}: ${p < 0 ? 'masked' : p.toFixed(3)}`}
                        />
                      );
                    }),
                  )}
                </div>
              </figure>
            ))}
          </div>
        </section>
      ) : null}

      {!tokState.data && !fwdState.data && !tokState.loading && !fwdState.loading ? (
        <EmptyState icon="atom" title="No computation yet" hint="Tokenize text or run a tiny forward pass to see real attention probabilities." />
      ) : null}
    </div>
  );
}
