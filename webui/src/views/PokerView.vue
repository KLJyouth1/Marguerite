<template>
  <div class="control-stage poker-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">{{ room ? room.name : '德州扑克' }}</h1>
        <p class="control-stage__subtitle">
          <template v-if="room">
            盲注 {{ room.small_blind }}/{{ room.big_blind }} · {{ streetLabel }} · 底池 {{ potTotal }}
          </template>
          <template v-else>游戏大厅 · 赌 Token — 多人同桌对战（你 + AI 座位，Token 为局内筹码）</template>
        </p>
      </div>
      <div class="control-stage__actions" v-if="room">
        <button class="btn btn--ghost" @click="refresh">
          <Icon name="refresh" :size="16" /><span>刷新</span>
        </button>
      </div>
    </header>

    <section v-if="error" class="poker-error">{{ error }}</section>

    <!-- 建桌 -->
    <section v-if="!room" class="poker-create">
      <input v-model="roomName" class="poker-input" placeholder="房间名" />
      <div class="poker-create__row">
        <input v-model="sb" class="poker-input" placeholder="小盲" />
        <input v-model="bb" class="poker-input" placeholder="大盲" />
      </div>
      <button class="btn btn--primary" :disabled="busy" @click="createAndStart">
        <Icon name="coins" :size="16" /><span>{{ busy ? '创建中…' : '创建房间并开局（1 人 + 2 AI）' }}</span>
      </button>
    </section>

    <template v-else>
      <!-- 公共牌 -->
      <section class="poker-board">
        <div class="poker-section-title">公共牌</div>
        <div class="poker-board__cards">
          <div v-for="(c, i) in boardSlots" :key="i" class="poker-card" :class="cardClass(c)">
            <template v-if="c.rank"><span class="poker-card__rank">{{ rankLabel(c.rank) }}</span><span class="poker-card__suit">{{ suitLabel(c.suit) }}</span></template>
            <template v-else>待发</template>
          </div>
        </div>
      </section>

      <!-- 玩家座位 -->
      <section class="poker-seats">
        <div v-for="p in room.players" :key="p.seat" class="poker-seat" :class="{ 'poker-seat--turn': isTurn(p.seat), 'poker-seat--folded': p.folded }">
          <div class="poker-seat__head">
            <span class="poker-seat__name">{{ p.name }}{{ p.seat === room.dealer_seat ? ' · 庄' : '' }}{{ p.is_human ? ' · 你' : '' }}</span>
            <span class="poker-seat__tokens">Token {{ p.tokens }}</span>
          </div>
          <div class="poker-seat__cards">
            <div v-for="(c, i) in seatCards(p)" :key="i" class="poker-card" :class="cardClass(c)">
              <template v-if="c.rank"><span class="poker-card__rank">{{ rankLabel(c.rank) }}</span><span class="poker-card__suit">{{ suitLabel(c.suit) }}</span></template>
              <template v-else>？</template>
            </div>
          </div>
          <div class="poker-seat__badges">
            <span v-if="p.folded" class="poker-badge poker-badge--fold">弃牌</span>
            <span v-if="p.all_in" class="poker-badge poker-badge--allin">全下</span>
            <span v-if="isTurn(p.seat)" class="poker-badge poker-badge--turn">行动中</span>
            <span v-if="p.committed > 0" class="poker-seat__committed">已投入 {{ p.committed }}</span>
          </div>
        </div>
      </section>

      <!-- 最近动作 / 结果 -->
      <section class="poker-log">
        <div v-if="room.last_action" class="poker-log__action">最近动作：{{ room.last_action }}</div>
        <div v-for="[seat, amount] in winners" :key="seat" class="poker-log__winner">
          {{ playerName(seat) }} 赢得 {{ amount }} Token
        </div>
      </section>

      <!-- 操作区 -->
      <section class="poker-actions">
        <template v-if="room.hand_over">
          <button class="btn btn--primary" :disabled="busy" @click="nextHand">下一局</button>
        </template>
        <template v-else-if="isMyTurn">
          <button class="btn btn--ghost" :disabled="busy" @click="act('fold')">弃牌</button>
          <button class="btn btn--ghost" :disabled="busy || toCall > 0" @click="act('check')">过牌</button>
          <button class="btn btn--primary" :disabled="busy || toCall <= 0" @click="act('call')">跟注 {{ toCall }}</button>
          <input v-model="raiseTo" class="poker-input poker-input--raise" :placeholder="`加注到 ≥${room.street_high + room.min_raise}`" />
          <button class="btn btn--primary" :disabled="busy || !raiseTo" @click="act('raise', Number(raiseTo))">加注</button>
          <button class="btn btn--secondary" :disabled="busy" @click="act('all_in')">全下</button>
        </template>
        <div v-else class="poker-waiting">等待 {{ currentName }} 行动…</div>
      </section>
    </template>
  </div>
</template>

<script setup lang="ts">
import { ref, computed } from 'vue'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'

const rpc = useRpcStore()

interface PokerCard { rank: string; suit: string }
interface PokerPlayer {
  seat: number; name: string; tokens: number; committed: number; street_committed: number;
  folded: boolean; all_in: boolean; hole: [PokerCard | null, PokerCard | null]; is_human: boolean;
}
interface PokerPot { amount: number; eligible: number[] }
interface PokerRoom {
  id: string; name: string; players: PokerPlayer[]; board: PokerCard[]; street: string;
  dealer_seat: number; current_seat: number; small_blind: number; big_blind: number;
  street_high: number; min_raise: number; pots: PokerPot[]; started: boolean; hand_over: boolean;
  winners: [number, number][]; last_action: string;
}

const RANK_LABEL: Record<string, string> = {
  Two: '2', Three: '3', Four: '4', Five: '5', Six: '6', Seven: '7', Eight: '8', Nine: '9',
  Ten: 'T', Jack: 'J', Queen: 'Q', King: 'K', Ace: 'A',
}
const SUIT_LABEL: Record<string, string> = { Spades: 's', Hearts: 'h', Diamonds: 'd', Clubs: 'c' }
const RED_SUIT = new Set(['Hearts', 'Diamonds'])
const STREET_LABEL: Record<string, string> = {
  Preflop: '翻前', Flop: '翻牌', Turn: '转牌', River: '河牌', Showdown: '摊牌',
}

function rankLabel(r: string): string { return RANK_LABEL[r] ?? r }
function suitLabel(s: string): string { return SUIT_LABEL[s] ?? s }
function cardClass(c: PokerCard): string[] {
  if (!c.rank) return ['poker-card--empty']
  return RED_SUIT.has(c.suit) ? ['poker-card--red'] : ['poker-card--black']
}

const room = ref<PokerRoom | null>(null)
const roomName = ref('茉莉牌局')
const sb = ref('5')
const bb = ref('10')
const raiseTo = ref('')
const busy = ref(false)
const error = ref('')

const human = computed(() => room.value?.players.find((p) => p.is_human))
const humanSeat = computed(() => human.value?.seat ?? -1)
const isMyTurn = computed(() => !!room.value && !room.value.hand_over && room.value.current_seat === humanSeat.value)
const toCall = computed(() => (room.value && human.value ? room.value.street_high - human.value.street_committed : 0))
const potTotal = computed(() => room.value ? room.value.pots.reduce((s, p) => s + p.amount, 0) : 0)
const streetLabel = computed(() => (room.value ? (STREET_LABEL[room.value.street] ?? room.value.street) : ''))
const winners = computed(() => room.value?.winners ?? [])
const currentName = computed(() => room.value ? (room.value.players[room.value.current_seat]?.name ?? '…') : '…')

const boardSlots = computed<PokerCard[]>(() => {
  const b = room.value?.board ?? []
  const out = [...b]
  while (out.length < 5) out.push({ rank: '', suit: '' } as PokerCard)
  return out
})

function seatCards(p: PokerPlayer): PokerCard[] {
  const show = room.value?.hand_over || p.is_human
  if (show && p.hole) return p.hole.map((c) => c ?? { rank: '', suit: '' } as PokerCard)
  return [{ rank: '', suit: '' } as PokerCard, { rank: '', suit: '' } as PokerCard]
}

function isTurn(seat: number): boolean {
  return !!room.value && !room.value.hand_over && room.value.current_seat === seat
}

function playerName(seat: number): string {
  return room.value?.players[seat]?.name ?? '座位' + seat
}

async function refresh() {
  if (!room.value) return
  try {
    const r = await rpc.call<PokerRoom>('poker.state', { roomId: room.value.id })
    room.value = r
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  }
}

async function createAndStart() {
  busy.value = true
  error.value = ''
  try {
    const created = await rpc.call<{ ok: boolean; roomId: string; error?: string }>('poker.create', {
      name: roomName.value.trim() || '茉莉牌局',
      smallBlind: parseInt(sb.value, 10) || 5,
      bigBlind: parseInt(bb.value, 10) || 10,
    })
    if (!created.ok) { error.value = created.error || '创建失败'; return }
    await rpc.call('poker.join', { roomId: created.roomId, playerName: '你', tokens: 1000, isHuman: true })
    await rpc.call('poker.join', { roomId: created.roomId, playerName: '莫斯', tokens: 1000, isHuman: false })
    await rpc.call('poker.join', { roomId: created.roomId, playerName: '星黎', tokens: 1000, isHuman: false })
    await rpc.call('poker.start', { roomId: created.roomId })
    await rpc.call('poker.auto', { roomId: created.roomId })
    room.value = await rpc.call<PokerRoom>('poker.state', { roomId: created.roomId })
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  } finally {
    busy.value = false
  }
}

async function act(action: string, amount?: number) {
  if (!room.value) return
  busy.value = true
  error.value = ''
  try {
    await rpc.call('poker.action', {
      roomId: room.value.id,
      seat: humanSeat.value,
      action,
      amount: amount ?? null,
    })
    await rpc.call('poker.auto', { roomId: room.value.id })
    await refresh()
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
    await refresh()
  } finally {
    busy.value = false
  }
}

async function nextHand() {
  if (!room.value) return
  busy.value = true
  error.value = ''
  try {
    await rpc.call('poker.start', { roomId: room.value.id })
    await rpc.call('poker.auto', { roomId: room.value.id })
    await refresh()
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  } finally {
    busy.value = false
  }
}
</script>

<style scoped>
.poker-stage { padding: 24px 28px; max-width: 1100px; margin: 0 auto; }
.poker-error { margin: 16px 0; padding: 10px 14px; background: color-mix(in srgb, var(--danger) 10%, var(--card)); border: 1px solid color-mix(in srgb, var(--danger) 30%, transparent); border-radius: var(--radius-sm); color: var(--danger); font-size: 12px; }
.poker-create { display: flex; flex-direction: column; gap: 8px; margin-top: 16px; padding: 14px; background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); max-width: 420px; }
.poker-create__row { display: flex; gap: 8px; }
.poker-input { padding: 9px 12px; border-radius: var(--radius-sm); border: 1px solid var(--hairline); background: var(--bg); color: var(--text); font-size: 13px; outline: none; }
.poker-input:focus { border-color: var(--accent); }
.poker-input--raise { width: 140px; }
.poker-board { margin-top: 16px; padding: 14px; background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); }
.poker-section-title { font-size: 11px; color: var(--text-dim); margin-bottom: 8px; }
.poker-board__cards { display: flex; gap: 6px; }
.poker-seats { margin-top: 16px; display: grid; grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); gap: 8px; }
.poker-seat { background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); padding: 12px; }
.poker-seat--turn { border-color: var(--accent); box-shadow: 0 0 0 1px var(--accent); }
.poker-seat--folded { opacity: 0.55; }
.poker-seat__head { display: flex; justify-content: space-between; align-items: center; }
.poker-seat__name { font-weight: 700; font-size: 13px; }
.poker-seat__tokens { font-size: 11px; color: var(--text-dim); }
.poker-seat__cards { display: flex; gap: 4px; margin-top: 8px; }
.poker-seat__badges { display: flex; gap: 4px; flex-wrap: wrap; margin-top: 8px; }
.poker-badge { font-size: 10px; padding: 2px 8px; border-radius: var(--radius-pill); }
.poker-badge--fold { background: color-mix(in srgb, var(--danger) 12%, var(--card)); color: var(--danger); }
.poker-badge--allin { background: color-mix(in srgb, var(--ok) 12%, var(--card)); color: var(--ok); }
.poker-badge--turn { background: color-mix(in srgb, var(--accent) 12%, var(--card)); color: var(--accent); }
.poker-seat__committed { font-size: 10px; color: var(--text-dim); }
.poker-log { margin-top: 16px; padding: 12px 14px; background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); font-size: 12px; }
.poker-log__action { color: var(--text-dim); }
.poker-log__winner { margin-top: 4px; font-weight: 600; color: var(--accent); }
.poker-actions { margin-top: 16px; padding: 14px; background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
.poker-waiting { font-size: 12px; color: var(--text-dim); }
.poker-card { width: 46px; height: 62px; border-radius: var(--radius-sm); background: var(--card); border: 1px solid var(--hairline); display: inline-flex; flex-direction: column; align-items: center; justify-content: center; font-family: var(--font-mono, monospace); }
.poker-card--empty { border-style: dashed; color: var(--text-dim); font-size: 10px; }
.poker-card--red { color: var(--danger); }
.poker-card--black { color: var(--text); }
.poker-card__rank { font-size: 15px; font-weight: 700; line-height: 1; }
.poker-card__suit { font-size: 11px; line-height: 1; }
</style>
