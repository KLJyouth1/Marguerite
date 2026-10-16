import type { ButtonHTMLAttributes, ReactNode } from 'react';
import { Icon, type IconName } from './icons';

type Variant = 'primary' | 'secondary' | 'ghost' | 'danger';
type Size = 'sm' | 'md';

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  size?: Size;
  icon?: IconName;
  /** When true and `icon` is set, hides label text and exposes an aria-label. */
  iconOnly?: boolean;
  children?: ReactNode;
}

/**
 * Semantic <button> wrapper (never a div with onClick — Bloom R6).
 * Icon-only usage requires an `aria-label` via the `aria-label` attribute.
 */
export function Button({
  variant = 'secondary',
  size = 'md',
  icon,
  iconOnly = false,
  children,
  className = '',
  type = 'button',
  ...rest
}: ButtonProps): JSX.Element {
  const classes = [
    'btn',
    `btn--${variant}`,
    `btn--${size}`,
    iconOnly ? 'btn--icon-only' : '',
    className,
  ]
    .filter(Boolean)
    .join(' ');

  return (
    <button type={type} className={classes} {...rest}>
      {icon ? <Icon name={icon} size={size === 'sm' ? 15 : 17} /> : null}
      {!iconOnly && children ? <span className="btn__label">{children}</span> : null}
    </button>
  );
}
