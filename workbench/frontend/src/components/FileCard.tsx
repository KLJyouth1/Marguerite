import { useState } from 'react';
import type {
  AskResult,
  FileCard as FileCardModel,
  FileDisplay,
  InteractResult,
  ModifyResult,
} from '../types/contract';
import { api } from '../api/client';
import { useAsync } from '../hooks/useAsync';
import { useWorkbenchStore } from '../store/workbenchStore';
import { Button } from './ui/Button';
import { Icon } from './ui/icons';
import { SafeHtml } from './ui/SafeHtml';
import { ErrorState } from './ui/States';
import { DiffView } from './DiffView';
import { FileSection } from './FileSection';

type OpenView = 'none' | 'output' | 'modify' | 'ask' | 'interact';

interface ChatMessage {
  role: 'user' | 'agent';
  text: string;
}

export interface FileCardProps {
  card: FileCardModel;
  onUpdate: (updated: FileCardModel) => void;
  onRemove: (fileId: string) => void;
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function FileCard({ card, onUpdate, onRemove }: FileCardProps): JSX.Element {
  const [view, setView] = useState<OpenView>('none');
  const [busySection, setBusySection] = useState<string | null>(null);

  const outputState = useAsync<FileDisplay>();
  const modifyState = useAsync<ModifyResult>();
  const askState = useAsync<AskResult>();
  const interactState = useAsync<InteractResult>();

  const [instruction, setInstruction] = useState('');
  const [question, setQuestion] = useState('');
  const [interactInput, setInteractInput] = useState('');
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [messages, setMessages] = useState<ChatMessage[]>([]);

  const pushToast = useWorkbenchStore((s) => s.pushToast);

  const toggleAction = (next: OpenView) => setView((prev) => (prev === next ? 'none' : next));

  const handleToggleSection = async (sectionId: string) => {
    setBusySection(sectionId);
    try {
      const updated = await api.collapseFile(card.file_id, [sectionId]);
      onUpdate(updated);
    } catch (err) {
      pushToast('error', err instanceof Error ? err.message : 'Failed to toggle section');
    } finally {
      setBusySection(null);
    }
  };

  const handleCollapseAll = async () => {
    try {
      const updated = await api.collapseFile(card.file_id, null);
      onUpdate(updated);
      pushToast('success', 'Folded long sections');
    } catch (err) {
      pushToast('error', err instanceof Error ? err.message : 'Failed to collapse');
    }
  };

  const handleOutput = () => {
    toggleAction('output');
    if (view !== 'output') {
      void outputState.run(() => api.getFileDisplay(card.file_id));
    }
  };

  const handleModify = async () => {
    if (!instruction.trim()) return;
    try {
      const res = await modifyState.run(() => api.modifyFile(card.file_id, instruction.trim()));
      pushToast(res.applied ? 'success' : 'info', res.applied ? 'Modification applied' : 'Previewed (not applied)');
    } catch (err) {
      pushToast('error', err instanceof Error ? err.message : 'Modify failed');
    }
  };

  const handleAsk = async () => {
    if (!question.trim()) return;
    try {
      await askState.run(() => api.askFile(card.file_id, question.trim()));
    } catch (err) {
      pushToast('error', err instanceof Error ? err.message : 'Ask failed');
    }
  };

  const handleInteract = async () => {
    const text = interactInput.trim();
    if (!text) return;
    setInteractInput('');
    setMessages((prev) => [...prev, { role: 'user', text }]);
    try {
      const res = await interactState.run(() =>
        api.interactFile(card.file_id, text, sessionId),
      );
      setSessionId(res.session_id);
      setMessages((prev) => [...prev, { role: 'agent', text: res.reply }]);
    } catch (err) {
      setMessages((prev) => [
        ...prev,
        { role: 'agent', text: err instanceof Error ? `Error: ${err.message}` : 'Error' },
      ]);
    }
  };

  return (
    <article className="filecard" aria-label={`File ${card.path}`}>
      <header className="filecard__head">
        <span className="filecard__glyph" aria-hidden="true">
          <Icon name="file" size={20} />
        </span>
        <div className="filecard__meta">
          <h3 className="filecard__path" title={card.path}>
            {card.path}
          </h3>
          <div className="filecard__facts">
            {card.language ? <span className="chip">{card.language}</span> : null}
            <span className="filecard__fact">{formatSize(card.size_bytes)}</span>
            <span className="filecard__fact filecard__fact--mono" title={card.sha256}>
              sha256:{card.sha256.slice(0, 12)}
            </span>
          </div>
        </div>
        <div className="filecard__tools">
          <Button
            variant={view === 'modify' ? 'primary' : 'ghost'}
            size="sm"
            icon="edit"
            onClick={() => toggleAction('modify')}
            aria-label="Modify file"
            title="Modify"
          />
          <Button
            variant={view === 'output' ? 'primary' : 'ghost'}
            size="sm"
            icon="eye"
            onClick={handleOutput}
            aria-label="Display rendered artifact"
            title="Display"
          />
          <Button
            variant={view === 'ask' ? 'primary' : 'ghost'}
            size="sm"
            icon="question"
            onClick={() => toggleAction('ask')}
            aria-label="Ask a question"
            title="Ask"
          />
          <Button
            variant={view === 'interact' ? 'primary' : 'ghost'}
            size="sm"
            icon="chat"
            onClick={() => toggleAction('interact')}
            aria-label="Open interactive chat"
            title="Interact"
          />
          <Button
            variant="ghost"
            size="sm"
            icon="collapse"
            onClick={handleCollapseAll}
            aria-label="Collapse long sections"
            title="Collapse all"
          />
          <Button
            variant="ghost"
            size="sm"
            icon="close"
            onClick={() => onRemove(card.file_id)}
            aria-label="Remove file card"
            title="Remove"
          />
        </div>
      </header>

      <div className="filecard__sections">
        {card.sections.length === 0 ? (
          <p className="filecard__nosections">This file was loaded without sections.</p>
        ) : (
          card.sections.map((section) => (
            <FileSection
              key={section.id}
              section={section}
              busy={busySection === section.id}
              onRequestToggle={handleToggleSection}
            />
          ))
        )}
      </div>

      {view === 'output' ? (
        <div className="filecard__detail">
          <div className="filecard__detail-head">
            <span>Rendered artifact · {outputState.data?.language ?? card.language ?? 'text'}</span>
            <Button variant="ghost" size="sm" icon="close" iconOnly aria-label="Close artifact" onClick={() => setView('none')} />
          </div>
          {outputState.loading ? (
            <p className="filecard__muted">Rendering artifact…</p>
          ) : outputState.error ? (
            <ErrorState title="Could not load artifact" message={outputState.error} />
          ) : outputState.data ? (
            <SafeHtml html={outputState.data.html} className="filecard__artifact" />
          ) : null}
        </div>
      ) : null}

      {view === 'modify' ? (
        <div className="filecard__detail">
          <div className="filecard__detail-head">
            <span>Modify · instruction</span>
            <Button variant="ghost" size="sm" icon="close" iconOnly aria-label="Close" onClick={() => setView('none')} />
          </div>
          <div className="filecard__form">
            <textarea
              className="input input--area"
              placeholder="Describe the change, e.g. 'rename the retry helper to withRetry and add a timeout'"
              value={instruction}
              onChange={(e) => setInstruction(e.target.value)}
              rows={3}
              aria-label="Modification instruction"
            />
            <div className="filecard__form-actions">
              <Button variant="primary" size="sm" icon="send" onClick={handleModify} disabled={modifyState.loading || !instruction.trim()}>
                {modifyState.loading ? 'Applying…' : 'Apply modification'}
              </Button>
            </div>
          </div>
          {modifyState.error ? (
            <ErrorState title="Modify failed" message={modifyState.error} />
          ) : modifyState.data ? (
            <div className="filecard__modify-result">
              <div className="filecard__badges">
                <span className={`verdict verdict--${modifyState.data.applied ? 'pass' : 'conditional'}`}>
                  {modifyState.data.applied ? 'Applied' : 'Preview only'}
                </span>
                <span className="chip chip--mono">backup:{modifyState.data.backup_ref}</span>
              </div>
              <DiffView diff={modifyState.data.diff} />
            </div>
          ) : null}
        </div>
      ) : null}

      {view === 'ask' ? (
        <div className="filecard__detail">
          <div className="filecard__detail-head">
            <span>Ask · question</span>
            <Button variant="ghost" size="sm" icon="close" iconOnly aria-label="Close" onClick={() => setView('none')} />
          </div>
          <div className="filecard__form">
            <input
              className="input"
              placeholder="Ask about this file…"
              value={question}
              onChange={(e) => setQuestion(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') void handleAsk();
              }}
              aria-label="Question"
            />
            <div className="filecard__form-actions">
              <Button variant="primary" size="sm" icon="send" onClick={handleAsk} disabled={askState.loading || !question.trim()}>
                {askState.loading ? 'Asking…' : 'Ask'}
              </Button>
            </div>
          </div>
          {askState.error ? (
            <ErrorState title="Ask failed" message={askState.error} />
          ) : askState.data ? (
            <div className="filecard__ask-result">
              <p className="filecard__answer">{askState.data.answer}</p>
              {askState.data.citations.length > 0 ? (
                <ul className="citations">
                  {askState.data.citations.map((c, i) => (
                    <li key={i} className="citation">
                      {c.source ? <span className="citation__source">{c.source}</span> : null}
                      {c.line !== undefined ? <span className="citation__line">line {c.line}</span> : null}
                      {c.text ? <span className="citation__text">{c.text}</span> : null}
                    </li>
                  ))}
                </ul>
              ) : null}
            </div>
          ) : null}
        </div>
      ) : null}

      {view === 'interact' ? (
        <div className="filecard__detail">
          <div className="filecard__detail-head">
            <span>Interact · multi-turn{sessionId ? ` · session ${sessionId.slice(0, 8)}` : ''}</span>
            <Button variant="ghost" size="sm" icon="close" iconOnly aria-label="Close" onClick={() => setView('none')} />
          </div>
          <div className="chat" aria-live="polite">
            {messages.length === 0 ? (
              <p className="filecard__muted">Start a conversation about this file. The session is kept server-side.</p>
            ) : (
              messages.map((m, i) => (
                <div key={i} className={`chat__row chat__row--${m.role}`}>
                  <span className="chat__role">{m.role === 'user' ? 'You' : 'Agent'}</span>
                  <p className="chat__text">{m.text}</p>
                </div>
              ))
            )}
            {interactState.loading ? <p className="filecard__muted">Agent is typing…</p> : null}
          </div>
          <div className="filecard__form filecard__form--inline">
            <input
              className="input"
              placeholder="Send a message…"
              value={interactInput}
              onChange={(e) => setInteractInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') void handleInteract();
              }}
              aria-label="Chat message"
            />
            <Button variant="primary" size="sm" icon="send" onClick={handleInteract} disabled={interactState.loading || !interactInput.trim()} aria-label="Send message">
              Send
            </Button>
          </div>
        </div>
      ) : null}
    </article>
  );
}
