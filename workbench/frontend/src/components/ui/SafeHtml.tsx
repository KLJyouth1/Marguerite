import { useMemo } from 'react';
import DOMPurify from 'dompurify';

/**
 * Renders backend-produced HTML (e.g. the syntax-highlighted file artifact)
 * after sanitizing it. Bloom R14: no `dangerouslySetInnerHTML` without DOMPurify.
 */
export function SafeHtml({ html, className }: { html: string; className?: string }): JSX.Element {
  const clean = useMemo(() => DOMPurify.sanitize(html, { USE_PROFILES: { html: true } }), [html]);
  return <div className={className} dangerouslySetInnerHTML={{ __html: clean }} />;
}
