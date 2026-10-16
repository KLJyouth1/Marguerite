import { useState, memo } from "react"
import { invoke } from "@tauri-apps/api/core"
import { Badge } from "../components/primitives"
import { btnGhostStyle, btnStyle, cardStyle, inputStyle, panelBox, panelSubtitle, panelTitle } from "../styles/panelStyles"


interface PokerCardView {
  rank: string;
  suit: string;
}

interface PokerPlayerView {
  seat: number;
  name: string;
  tokens: number;
  committed: number;
  street_committed: number;
  folded: boolean;
  all_in: boolean;
  hole: [PokerCardView | null, PokerCardView | null] | null;
  is_human: boolean;
}

interface PokerPotView {
  amount: number;
  eligible: number[];
}

interface PokerRoomView {
  id: string;
  name: string;
  players: PokerPlayerView[];
  board: PokerCardView[];
  street: string;
  dealer_seat: number;
  current_seat: number;
  small_blind: number;
  big_blind: number;
  street_high: number;
  min_raise: number;
  pots: PokerPotView[];
  started: boolean;
  hand_over: boolean;
  winners: [number, number][];
  last_action: string;
}

const POKER_RANK_LABEL: Record<string, string> = {
  Two: "2", Three: "3", Four: "4", Five: "5", Six: "6", Seven: "7",
  Eight: "8", Nine: "9", Ten: "T", Jack: "J", Queen: "Q", King: "K", Ace: "A",
};

const POKER_SUIT_LABEL: Record<string, string> = {
  Spades: "s", Hearts: "h", Diamonds: "d", Clubs: "c",
};

const POKER_RED_SUIT = new Set(["Hearts", "Diamonds"]);

const POKER_STREET_LABEL: Record<string, string> = {
  Preflop: "翻前", Flop: "翻牌", Turn: "转牌", River: "河牌", Showdown: "摊牌",
};

function PokerCardFaceImpl({ card }: { card: PokerCardView | null }) {
  if (!card) {
    return (
      <div style={{
        width: 46, height: 62, borderRadius: 6, border: "1px dashed var(--surface-border)",
        display: "inline-flex", alignItems: "center", justifyContent: "center",
        color: "var(--text-tertiary)", fontSize: 10,
      }}>待发</div>
    );
  }
  const red = POKER_RED_SUIT.has(card.suit);
  return (
    <div style={{
      width: 46, height: 62, borderRadius: 6, background: "#FFFFFF", border: "1px solid var(--surface-border)",
      display: "inline-flex", flexDirection: "column", alignItems: "center", justifyContent: "center",
      color: red ? "#DC2626" : "#1F2937", fontWeight: 700, boxShadow: "var(--shadow-sm)",
      fontFamily: "ui-monospace, monospace",
    }}>
      <span style={{ fontSize: 15 }}>{POKER_RANK_LABEL[card.rank] ?? card.rank}</span>
      <span style={{ fontSize: 11 }}>{POKER_SUIT_LABEL[card.suit] ?? card.suit}</span>
    </div>
  );
}

function PokerPanelImpl() {
  const [room, setRoom] = useState<PokerRoomView | null>(null);
  const [roomName, setRoomName] = useState("茉莉牌局");
  const [sb, setSb] = useState("5");
  const [bb, setBb] = useState("10");
  const [raiseTo, setRaiseTo] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = async (rid: string) => {
    try {
      const r = await invoke<PokerRoomView>("poker_state", { roomId: rid });
      setRoom(r);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const createAndStart = async () => {
    setBusy(true);
    setError(null);
    try {
      const rid = await invoke<string>("poker_create_room", {
        name: roomName.trim() || "茉莉牌局",
        smallBlind: parseInt(sb, 10) || 5,
        bigBlind: parseInt(bb, 10) || 10,
      });
      await invoke("poker_join_room", { roomId: rid, playerName: "你", tokens: 1000, isHuman: true });
      await invoke("poker_join_room", { roomId: rid, playerName: "莫斯", tokens: 1000, isHuman: false });
      await invoke("poker_join_room", { roomId: rid, playerName: "星黎", tokens: 1000, isHuman: false });
      await invoke("poker_start_room", { roomId: rid });
      await invoke("poker_auto", { roomId: rid });
      await refresh(rid);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const act = async (action: string, amount?: number) => {
    if (!room) return;
    setBusy(true);
    setError(null);
    try {
      await invoke("poker_action", {
        roomId: room.id,
        seat: room.players.find((p) => p.is_human)?.seat ?? 0,
        action,
        amount: amount ?? null,
      });
      await invoke("poker_auto", { roomId: room.id });
      await refresh(room.id);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      await refresh(room.id);
    } finally {
      setBusy(false);
    }
  };

  const nextHand = async () => {
    if (!room) return;
    setBusy(true);
    setError(null);
    try {
      await invoke("poker_start_room", { roomId: room.id });
      await invoke("poker_auto", { roomId: room.id });
      await refresh(room.id);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const human = room?.players.find((p) => p.is_human);
  const humanSeat = human?.seat ?? -1;
  const isMyTurn = !!room && !room.hand_over && room.current_seat === humanSeat;
  const toCall = room && human ? room.street_high - human.street_committed : 0;
  const potTotal = room ? room.pots.reduce((s, p) => s + p.amount, 0) : 0;

  if (!room) {
    return (
      <div style={panelBox}>
        <div style={panelTitle}>德州扑克</div>
        <div style={panelSubtitle}>游戏大厅 · 赌 Token — 多人同桌对战（你 + AI 座位），Token 为局内筹码（本地房间模型，跨机联机需 relay 服务）</div>
        <div style={{ ...cardStyle, display: "flex", flexDirection: "column", gap: 8, maxWidth: 420 }}>
          <input style={inputStyle} placeholder="房间名" value={roomName} onChange={(e) => setRoomName(e.target.value)} />
          <div style={{ display: "flex", gap: 8 }}>
            <input style={inputStyle} placeholder="小盲" value={sb} onChange={(e) => setSb(e.target.value)} />
            <input style={inputStyle} placeholder="大盲" value={bb} onChange={(e) => setBb(e.target.value)} />
          </div>
          <button style={btnStyle} onClick={createAndStart} disabled={busy}>
            {busy ? "创建中…" : "创建房间并开局（1 人 + 2 AI）"}
          </button>
          {error && <div style={{ color: "#ef4444", fontSize: 12 }}>{error}</div>}
        </div>
      </div>
    );
  }

  return (
    <div style={panelBox}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
        <div>
          <div style={panelTitle}>{room.name}</div>
          <div style={panelSubtitle}>
            盲注 {room.small_blind}/{room.big_blind} · 阶段 {POKER_STREET_LABEL[room.street] ?? room.street} · 底池 {potTotal}
          </div>
        </div>
        <button style={btnGhostStyle} onClick={() => refresh(room.id)} disabled={busy}>刷新</button>
      </div>

      {error && <div style={{ ...cardStyle, color: "#ef4444", fontSize: 12, marginTop: 8 }}>{error}</div>}

      {/* 公共牌 */}
      <div style={{ ...cardStyle, marginTop: 12 }}>
        <div style={{ fontSize: 11, color: "var(--text-secondary)", marginBottom: 8 }}>公共牌</div>
        <div style={{ display: "flex", gap: 6 }}>
          {room.board.map((c, i) => <PokerCardFace key={i} card={c} />)}
          {Array.from({ length: Math.max(0, 5 - room.board.length) }).map((_, i) => <PokerCardFace key={"e" + i} card={null} />)}
        </div>
      </div>

      {/* 玩家座位 */}
      <div style={{ marginTop: 12, display: "grid", gridTemplateColumns: "repeat(auto-fill, minmax(200px, 1fr))", gap: 8 }}>
        {room.players.map((p) => {
          const showHole = room.hand_over || p.is_human;
          const isTurn = !room.hand_over && room.current_seat === p.seat;
          return (
            <div key={p.seat} style={{
              ...cardStyle,
              border: isTurn ? "2px solid var(--accent-primary)" : undefined,
              opacity: p.folded ? 0.55 : 1,
            }}>
              <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                <span style={{ fontWeight: 700, fontSize: 13 }}>{p.name}{p.seat === room.dealer_seat ? " · 庄" : ""}{p.is_human ? " · 你" : ""}</span>
                <span style={{ fontSize: 11, color: "var(--text-secondary)" }}>Token {p.tokens}</span>
              </div>
              <div style={{ display: "flex", gap: 4, marginTop: 6 }}>
                {(p.hole && showHole ? p.hole : [null, null]).map((c, i) => <PokerCardFace key={i} card={c} />)}
              </div>
              <div style={{ display: "flex", gap: 4, marginTop: 6, flexWrap: "wrap" }}>
                {p.folded && <Badge ok={false} label="弃牌" />}
                {p.all_in && <Badge ok={true} label="全下" />}
                {isTurn && <Badge ok={true} label="行动中" />}
                {p.committed > 0 && <span style={{ fontSize: 10, color: "var(--text-secondary)" }}>已投入 {p.committed}</span>}
              </div>
            </div>
          );
        })}
      </div>

      {/* 最近动作 + 结果 */}
      <div style={{ ...cardStyle, marginTop: 12, fontSize: 12 }}>
        {room.last_action && <div style={{ color: "var(--text-secondary)" }}>最近动作：{room.last_action}</div>}
        {room.hand_over && room.winners.map(([seat, amount]) => (
          <div key={seat} style={{ marginTop: 4, fontWeight: 600, color: "var(--accent-primary)" }}>
            {room.players[seat]?.name ?? "座位" + seat} 赢得 {amount} Token
          </div>
        ))}
      </div>

      {/* 操作区 */}
      <div style={{ ...cardStyle, marginTop: 12 }}>
        {room.hand_over ? (
          <button style={btnStyle} onClick={nextHand} disabled={busy}>下一局</button>
        ) : isMyTurn ? (
          <div style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
            <button style={btnStyle} onClick={() => act("fold")} disabled={busy}>弃牌</button>
            <button style={btnStyle} onClick={() => act("check")} disabled={busy || toCall > 0}>过牌</button>
            <button style={btnStyle} onClick={() => act("call")} disabled={busy || toCall <= 0}>跟注 {toCall}</button>
            <input
              style={{ ...inputStyle, width: 110 }}
              placeholder={`加注到 ≥${room.street_high + room.min_raise}`}
              value={raiseTo}
              onChange={(e) => setRaiseTo(e.target.value)}
            />
            <button style={btnStyle} onClick={() => act("raise", parseInt(raiseTo, 10))} disabled={busy || !raiseTo}>加注</button>
            <button style={{ ...btnStyle, background: "var(--status-warning)" }} onClick={() => act("all_in")} disabled={busy}>全下</button>
          </div>
        ) : (
          <div style={{ fontSize: 12, color: "var(--text-secondary)" }}>
            等待 {room.players[room.current_seat]?.name ?? "…"} 行动…
          </div>
        )}
      </div>
    </div>
  );
}

// ───────────── 关系图谱（KB 2.0 · kb.graph）─────────────


// === B3: React.memo 包装 — 父级无关状态变更不再触发本组件重渲染 ===
export const PokerCardFace = memo(PokerCardFaceImpl);
export const PokerPanel = memo(PokerPanelImpl);
