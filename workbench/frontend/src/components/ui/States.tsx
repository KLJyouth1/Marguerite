import type { ReactNode } from 'react';
import { Icon, type IconName } from './icons';

export function Spinner({ size = 18 }: { size?: number }): JSX.Element {
  return (
    <svg
      className="spinner"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
      focusable="false"
    >
      <circle cx="12" cy="12" r="9" stroke="currentColor" strokeWidth="2.5" opacity="0.2" />
      <path
        d="M21 12a9 9 0 0 0-9-9"
        stroke="currentColor"
        strokeWidth="2.5"
        strokeLinecap="round"
      />
    </svg>
  );
}

export function LoadingState({ label = 'Loading…' }: { label?: string }): JSX.Element {
  return (
    <div className="state state--loading" role="status" aria-live="polite">
      <Spinner size={22} />
      <span>{label}</span>
    </div>
  );
}

export function EmptyState({
  icon = 'spark',
  title,
  hint,
  action,
}: {
  icon?: IconName;
  title: string;
  hint?: string;
  action?: ReactNode;
}): JSX.Element {
  return (
    <div className="state state--empty">
      <span className="state__glyph" aria-hidden="true">
        <Icon name={icon} size={26} />
      </span>
      <p className="state__title">{title}</p>
      {hint ? <p className="state__hint">{hint}</p> : null}
      {action ? <div className="state__action">{action}</div> : null}
    </div>
  );
}

export function ErrorState({
  title = 'Something went wrong',
  message,
  onRetry,
}: {
  title?: string;
  message?: string;
  onRetry?: () => void;
}): JSX.Element {
  return (
    <div className="state state--error" role="alert">
      <span className="state__glyph" aria-hidden="true">
        <Icon name="alert" size={26} />
      </span>
      <p className="state__title">{title}</p>
      {message ? <p className="state__hint">{message}</p> : null}
      {onRetry ? (
        <div className="state__action">
          <button type="button" className="btn btn--secondary btn--sm" onClick={onRetry}>
            Retry
          </button>
        </div>
      ) : null}
    </div>
  );
}
