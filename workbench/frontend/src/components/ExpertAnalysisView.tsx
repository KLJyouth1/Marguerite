import type { ExpertAnalysis } from '../types/contract';
import { Icon } from './ui/icons';

const VERDICT_LABEL: Record<ExpertAnalysis['verdict'], string> = {
  pass: 'Pass',
  conditional: 'Conditional',
  fail: 'Fail',
};

const INTENSITY_LABEL: Record<string, string> = {
  minimal: '极简',
  standard: '标准',
  extreme: '极致',
};

const WORKING_LABEL: Record<string, string> = {
  ask: '咨询',
  plan: '方案',
  doit: '执行',
};

export function ExpertAnalysisView({ analysis }: { analysis: ExpertAnalysis }): JSX.Element {
  const pct = Math.round(analysis.confidence * 100);
  const showMode = Boolean(analysis.intensity && analysis.working);
  return (
    <article className="expert-analysis" aria-label={`Analysis by ${analysis.role}`}>
      <div className="expert-analysis__head">
        <span className={`verdict verdict--${analysis.verdict}`}>
          <Icon name={analysis.verdict === 'pass' ? 'check' : analysis.verdict === 'fail' ? 'alert' : 'question'} size={14} />
          {VERDICT_LABEL[analysis.verdict]}
        </span>
        {showMode ? (
          <span className="expert-analysis__mode" aria-label={`Mode ${analysis.mode}`}>
            <span className="chip expert-analysis__mode-chip">{INTENSITY_LABEL[analysis.intensity as string]}</span>
            <span className="chip expert-analysis__mode-chip">{WORKING_LABEL[analysis.working as string]}</span>
          </span>
        ) : null}
        <div className="expert-analysis__confidence" aria-label={`Confidence ${pct}%`}>
          <span className="expert-analysis__conf-label">Confidence</span>
          <span className="expert-analysis__conf-bar">
            <span className="expert-analysis__conf-fill" style={{ width: `${pct}%` }} />
          </span>
          <span className="expert-analysis__conf-val">{pct}%</span>
        </div>
      </div>

      {analysis.notes && analysis.notes.length > 0 ? (
        <ul className="expert-analysis__notes">
          {analysis.notes.map((n, i) => (
            <li key={i}>{n}</li>
          ))}
        </ul>
      ) : null}

      <div className="expert-analysis__cols">
        <section className="expert-analysis__col">
          <h4 className="expert-analysis__col-title">Findings</h4>
          {analysis.findings.length === 0 ? (
            <p className="filecard__muted">No findings.</p>
          ) : (
            <ul className="expert-list">
              {analysis.findings.map((f, i) => (
                <li key={i}>{f}</li>
              ))}
            </ul>
          )}
        </section>
        <section className="expert-analysis__col">
          <h4 className="expert-analysis__col-title">Recommendations</h4>
          {analysis.recommendations.length === 0 ? (
            <p className="filecard__muted">No recommendations.</p>
          ) : (
            <ul className="expert-list">
              {analysis.recommendations.map((r, i) => (
                <li key={i}>{r}</li>
              ))}
            </ul>
          )}
        </section>
      </div>

      {analysis.plan && analysis.plan.length > 0 ? (
        <section className="expert-analysis__plan">
          <h4 className="expert-analysis__col-title">实施方案（Plan）</h4>
          <ol className="expert-plan">
            {analysis.plan.map((step) => (
              <li key={step.step} className="expert-plan__item">
                <span className="expert-plan__num">{step.step}</span>
                <span className="expert-plan__action">{step.action}</span>
                {step.milestone ? <span className="chip expert-plan__milestone">{step.milestone}</span> : null}
              </li>
            ))}
          </ol>
        </section>
      ) : null}

      {analysis.execution ? (
        <section className="expert-analysis__exec">
          <h4 className="expert-analysis__col-title">执行意图（DoIt）</h4>
          <ul className="expert-list">
            {analysis.execution.intent.map((it, i) => (
              <li key={i}>{it.action}</li>
            ))}
          </ul>
          <p className="expert-analysis__exec-note">
            <Icon name={analysis.execution.dispatched ? 'check' : 'question'} size={13} />
            {analysis.execution.note}
            {analysis.execution.dispatched ? '（已实际派发）' : '（未实际派发）'}
          </p>
        </section>
      ) : null}
    </article>
  );
}
