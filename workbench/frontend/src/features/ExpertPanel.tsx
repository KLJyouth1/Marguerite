import { useEffect, useState } from 'react';
import type { ExpertAnalysis, ExpertChatResult, ExpertMode, ExpertRole, ExpertTeam } from '../types/contract';
import { api } from '../api/client';
import { useAsync } from '../hooks/useAsync';
import { useWorkbenchStore } from '../store/workbenchStore';
import { Button } from '../components/ui/Button';
import { Icon } from '../components/ui/icons';
import { EmptyState, ErrorState, LoadingState } from '../components/ui/States';
import { ExpertAnalysisView } from '../components/ExpertAnalysisView';

const DEFAULT_PAYLOAD = '{\n  "subject": "Add rate limiting to the auth endpoint",\n  "context": "Public API, token-based"\n}';

// Chinese @mention alias for each team — mirrors the backend _TEAM_ALIASES so a
// click inserts a token the server resolves to the team lead.
const TEAM_MENTION: Record<string, string> = {
  deck_craft: 'PPT',
  fullstack_craft: '全栈',
  strategy_council: '战略',
  data_insight: '数据智能',
  legal_guard: '法务',
  invest_lab: '投研',
  game_forge: '游戏',
  product_craft: '产品',
};

const INTENSITY_LABEL: Record<string, string> = { minimal: '极简', standard: '标准', extreme: '极致' };
const WORKING_LABEL: Record<string, string> = { ask: '咨询', plan: '方案', doit: '执行' };

function teamMentionToken(team: ExpertTeam): string {
  return TEAM_MENTION[team.team] ?? team.team;
}

export function ExpertPanel(): JSX.Element {
  const modesState = useAsync<ExpertMode[]>();
  const teamsState = useAsync<ExpertTeam[]>();
  const expertsState = useAsync<ExpertRole[]>();

  const [mode, setMode] = useState<string>('standard_ask');
  const [selected, setSelected] = useState<ExpertRole | null>(null);
  const [payload, setPayload] = useState(DEFAULT_PAYLOAD);

  const [chatText, setChatText] = useState('');
  const [chatState, setChatState] = useState<{ loading: boolean; data?: ExpertChatResult; error?: string }>({ loading: false });

  const analysisState = useAsync<ExpertAnalysis>();
  const pushToast = useWorkbenchStore((s) => s.pushToast);

  useEffect(() => {
    void modesState.run(() => api.getExpertModes());
    void teamsState.run(() => api.getExpertTeams());
    void expertsState.run(() => api.getExperts());
  }, [modesState, teamsState, expertsState]);

  const insertMention = (token: string) => {
    setChatText((prev) => {
      const sep = prev.length === 0 || prev.endsWith(' ') || prev.endsWith('\n') ? '' : ' ';
      return `${prev}${sep}@${token} `;
    });
  };

  const handleSendChat = async () => {
    const text = chatText.trim();
    if (!text) return;
    setChatState({ loading: true });
    try {
      const res = await api.chatExperts(text, mode, selected?.role ?? null);
      setChatState({ loading: false, data: res });
      const n = res.mentioned.length;
      pushToast('success', n > 0 ? `已引用 ${n} 个专家团并分析` : '对话已发送（未识别到 @专家团）');
    } catch (err) {
      setChatState({ loading: false, error: err instanceof Error ? err.message : 'Chat failed' });
    }
  };

  const handleAnalyze = async () => {
    if (!selected) return;
    let parsed: Record<string, unknown>;
    try {
      parsed = JSON.parse(payload) as Record<string, unknown>;
    } catch {
      pushToast('error', 'Payload is not valid JSON');
      return;
    }
    try {
      const res = await analysisState.run(() => api.analyzeExpert(selected.role, parsed, mode));
      pushToast('success', `${selected.title} 分析完成（${MODE_LABEL_SAFE(mode)}）`);
      void res;
    } catch (err) {
      pushToast('error', err instanceof Error ? err.message : 'Analysis failed');
    }
  };

  // Lay the 9 modes out as a 3 (intensity) × 3 (working) matrix.
  const intensityOrder = ['minimal', 'standard', 'extreme'];
  const workingOrder = ['ask', 'plan', 'doit'];
  const modesByKey = new Map((modesState.data ?? []).map((m) => [`${m.intensity}_${m.working}`, m]));

  return (
    <div className="feature">
      <p className="feature__lede">
        选择专家团并在对话框中 <code>@引用</code> 它们，或选中单个角色提交结构化分析。所有分析在三种强度（极简 / 标准 / 极致）与三种工作模式（咨询 / 方案 / 执行）的组合下进行 —— 旧有的 bloom 等模式已重新规划升级为这套 3×3 矩阵。
      </p>

      {/* ---- 3×3 mode matrix ---- */}
      <section className="mode-matrix" aria-label="专家模式（3×3）">
        <header className="mode-matrix__head">
          <Icon name="spark" size={18} />
          <div>
            <h3 className="mode-matrix__title">专家模式</h3>
            <p className="analyze-box__hint">强度（行）× 工作模式（列）= 9 种组合，选中后用于对话引用与单角色分析。</p>
          </div>
        </header>
        {modesState.loading && !modesState.data ? <LoadingState label="Loading modes…" /> : null}
        {modesState.error && !modesState.data ? (
          <ErrorState title="Could not load modes" message={modesState.error} onRetry={() => void modesState.run(() => api.getExpertModes())} />
        ) : null}
        {modesState.data ? (
          <div className="mode-matrix__grid" role="grid">
            <div className="mode-matrix__corner" aria-hidden="true" />
            {workingOrder.map((w) => (
              <div key={w} className="mode-matrix__colhead">{WORKING_LABEL[w]}</div>
            ))}
            {intensityOrder.map((it) => (
              <div key={it} className="mode-matrix__row" role="row">
                <div className="mode-matrix__rowhead">{INTENSITY_LABEL[it]}</div>
                {workingOrder.map((w) => {
                  const m = modesByKey.get(`${it}_${w}`);
                  const id = `${it}_${w}`;
                  const active = mode === id;
                  return (
                    <button
                      key={w}
                      type="button"
                      role="gridcell"
                      className={`mode-cell ${active ? 'mode-cell--active' : ''}`}
                      aria-pressed={active}
                      title={m?.description ?? id}
                      onClick={() => setMode(id)}
                    >
                      <span className="mode-cell__label">{m?.label_zh ?? id}</span>
                      <span className="mode-cell__desc">{m?.description}</span>
                    </button>
                  );
                })}
              </div>
            ))}
          </div>
        ) : null}
      </section>

      {/* ---- team selector (the "page to pick expert teams") ---- */}
      <section className="team-select" aria-label="专家团选择">
        <header className="mode-matrix__head">
          <Icon name="brain" size={18} />
          <div>
            <h3 className="mode-matrix__title">专家团</h3>
            <p className="analyze-box__hint">点击「引用此团」把对应 @别名 送入下方对话框；点击成员可选中并做单角色分析。</p>
          </div>
        </header>

        {teamsState.loading && !teamsState.data ? <LoadingState label="Loading teams…" /> : null}
        {teamsState.error && !teamsState.data ? (
          <ErrorState title="Could not load teams" message={teamsState.error} onRetry={() => void teamsState.run(() => api.getExpertTeams())} />
        ) : null}

        {teamsState.data && teamsState.data.length === 0 ? (
          <EmptyState icon="brain" title="No expert teams available" hint="The backend returned an empty team list." />
        ) : null}

        {teamsState.data ? (
          <div className="team-grid">
            {teamsState.data.map((team) => {
              const token = teamMentionToken(team);
              const isCore = team.team === 'core';
              return (
                <article key={team.team} className="team-card">
                  <header className="team-card__head">
                    <span className="expert-card__glyph" aria-hidden="true"><Icon name="brain" size={16} /></span>
                    <div className="team-card__heading">
                      <h4 className="team-card__name">{team.name}</h4>
                      <span className="chip chip--mono">{team.category}</span>
                    </div>
                    <Button
                      variant="secondary"
                      size="sm"
                      icon="plus"
                      disabled={isCore}
                      onClick={() => insertMention(token)}
                      aria-label={`引用 ${team.name}`}
                    >
                      {isCore ? '核心' : '引用此团'}
                    </Button>
                  </header>

                  <div className="team-card__members">
                    {team.members.map((mem) => {
                      const roleObj = (expertsState.data ?? []).find((r) => r.role === mem.role) ?? null;
                      const isSel = selected?.role === mem.role;
                      return (
                        <button
                          key={mem.role}
                          type="button"
                          className={`member-chip ${isSel ? 'member-chip--active' : ''} ${mem.kind === 'lead' ? 'member-chip--lead' : ''}`}
                          onClick={() => setSelected(roleObj)}
                          aria-pressed={isSel}
                        >
                          {mem.kind === 'lead' ? <Icon name="check" size={12} /> : null}
                          {mem.title}
                        </button>
                      );
                    })}
                  </div>
                </article>
              );
            })}
          </div>
        ) : null}
      </section>

      {/* ---- dialog box: @mention chat ---- */}
      <section className="analyze-box" aria-label="专家对话（@引用）">
        <header className="analyze-box__head">
          <Icon name="chat" size={18} />
          <div>
            <h3>对话引用</h3>
            <p className="analyze-box__hint">在下方输入并以 @团队别名 引用专家团（如 {Object.values(TEAM_MENTION).slice(0, 4).map((t) => `@${t}`).join(' ')} …），回车或点击发送。</p>
          </div>
        </header>

        <div className="mention-hints">
          {Object.values(TEAM_MENTION).map((t) => (
            <button key={t} type="button" className="chip chip--button" onClick={() => insertMention(t)}>@{t}</button>
          ))}
        </div>

        <textarea
          className="input input--area"
          value={chatText}
          onChange={(e) => setChatText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) void handleSendChat();
          }}
          rows={4}
          placeholder="例如：@数据智能 帮我评估一下这个用户行为分析方案的可行性"
          aria-label="专家对话输入"
          spellCheck={false}
        />
        <div className="filecard__form-actions">
          <Button variant="primary" icon="send" onClick={() => void handleSendChat()} disabled={chatState.loading || !chatText.trim()}>
            {chatState.loading ? '分析中…' : '发送'}
          </Button>
        </div>

        {chatState.error ? <ErrorState title="对话分析失败" message={chatState.error} /> : null}
        {chatState.data ? (
          <div className="chat-result">
            <div className="chat-result__meta">
              <span className="chip expert-analysis__mode-chip">{MODE_LABEL_SAFE(chatState.data.mode)}</span>
              {chatState.data.mentioned.length > 0 ? (
                <span className="chat-result__mentioned">
                  {chatState.data.mentioned.map((m) => (
                    <span key={m} className="chip chip--mono">{m}</span>
                  ))}
                </span>
              ) : (
                <span className="filecard__muted">未识别到 @专家团（可作为默认角色分析）。</span>
              )}
            </div>
            {chatState.data.analyses.map((a, i) => (
              <ExpertAnalysisView key={`${a.role}-${i}`} analysis={a} />
            ))}
          </div>
        ) : null}
      </section>

      {/* ---- single-role analyze console ---- */}
      {selected ? (
        <section className="analyze-box" aria-label="单角色分析">
          <header className="analyze-box__head">
            <Icon name="spark" size={18} />
            <div>
              <h3>分析：{selected.title}</h3>
              <p className="analyze-box__hint">提交 JSON 载荷，按当前模式（{MODE_LABEL_SAFE(mode)}）运行结构化分析。</p>
            </div>
          </header>
          <textarea
            className="input input--area"
            value={payload}
            onChange={(e) => setPayload(e.target.value)}
            rows={6}
            aria-label="Analysis payload (JSON)"
            spellCheck={false}
          />
          <div className="filecard__form-actions">
            <Button variant="primary" icon="spark" onClick={() => void handleAnalyze()} disabled={analysisState.loading}>
              {analysisState.loading ? '分析中…' : '运行分析'}
            </Button>
          </div>
          {analysisState.error ? <ErrorState title="分析失败" message={analysisState.error} /> : null}
          {analysisState.data ? <ExpertAnalysisView analysis={analysisState.data} /> : null}
        </section>
      ) : null}
    </div>
  );
}

function MODE_LABEL_SAFE(id: string): string {
  const [it, w] = id.split('_');
  const i = INTENSITY_LABEL[it] ?? it;
  const k = WORKING_LABEL[w] ?? w;
  return `${i}·${k}`;
}
