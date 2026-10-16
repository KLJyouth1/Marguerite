import type { ComponentType } from 'react';
import type { FileSection as FileSectionModel, SectionKind } from '../types/contract';
import { Icon } from './ui/icons';

export interface SectionViewProps {
  section: FileSectionModel;
}

/**
 * Extensibility pattern: each section `kind` resolves to a body renderer via
 * this registry. Adding a new kind = adding one entry; no branching churn.
 */
export const sectionRegistry: Record<SectionKind, ComponentType<SectionViewProps>> = {
  heading: ({ section }) => (
    <pre className="section__code section__code--heading" aria-label="Section body">
      {section.full ?? section.preview}
    </pre>
  ),
  function: ({ section }) => (
    <pre className="section__code section__code--code" aria-label="Section body">
      {section.full ?? section.preview}
    </pre>
  ),
  class: ({ section }) => (
    <pre className="section__code section__code--code" aria-label="Section body">
      {section.full ?? section.preview}
    </pre>
  ),
  block: ({ section }) => (
    <pre className="section__code" aria-label="Section body">
      {section.full ?? section.preview}
    </pre>
  ),
  raw: ({ section }) => (
    <pre className="section__code section__code--raw" aria-label="Section body">
      {section.full ?? section.preview}
    </pre>
  ),
};

const KIND_LABEL: Record<SectionKind, string> = {
  heading: 'Heading',
  function: 'Function',
  class: 'Class',
  block: 'Block',
  raw: 'Raw',
};

export interface FileSectionProps {
  section: FileSectionModel;
  busy: boolean;
  onRequestToggle: (sectionId: string) => void;
}

export function FileSection({ section, busy, onRequestToggle }: FileSectionProps): JSX.Element {
  const Body = sectionRegistry[section.kind];
  return (
    <article className={`section ${section.collapsed ? 'section--collapsed' : 'section--open'}`}>
      <header className="section__head">
        <span className={`section__badge section__badge--${section.kind}`}>{KIND_LABEL[section.kind]}</span>
        <h4 className="section__title">{section.title ?? `lines ${section.start_line}–${section.end_line}`}</h4>
        <span className="section__range" aria-label="Line range">
          {section.start_line}–{section.end_line}
        </span>
        <button
          type="button"
          className="section__toggle"
          disabled={busy}
          aria-expanded={!section.collapsed}
          aria-label={section.collapsed ? 'Expand section' : 'Collapse section'}
          onClick={() => onRequestToggle(section.id)}
        >
          {busy ? <span className="section__spinner" aria-hidden="true" /> : <Icon name={section.collapsed ? 'chevronRight' : 'chevronDown'} size={16} />}
        </button>
      </header>
      {!section.collapsed ? (
        <div className="section__body">
          <Body section={section} />
        </div>
      ) : (
        <pre className="section__preview" aria-label="Section preview">
          {section.preview}
        </pre>
      )}
    </article>
  );
}
