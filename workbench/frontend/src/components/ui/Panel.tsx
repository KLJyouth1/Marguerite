import type { ReactNode } from 'react';

export interface PanelProps {
  /** Section heading. */
  title: string;
  /** Short supporting line shown under the title. */
  subtitle?: string;
  /** Optional icon shown before the title. */
  icon?: ReactNode;
  /** Right-aligned actions in the header. */
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
}

/** Card-shaped content region with a consistent header (Bloom tokenized). */
export function Panel({ title, subtitle, icon, actions, children, className = '' }: PanelProps): JSX.Element {
  return (
    <section className={`panel ${className}`.trim()} aria-label={title}>
      <header className="panel__head">
        <div className="panel__heading">
          {icon ? <span className="panel__icon" aria-hidden="true">{icon}</span> : null}
          <div>
            <h2 className="panel__title">{title}</h2>
            {subtitle ? <p className="panel__subtitle">{subtitle}</p> : null}
          </div>
        </div>
        {actions ? <div className="panel__actions">{actions}</div> : null}
      </header>
      <div className="panel__body">{children}</div>
    </section>
  );
}
