/**
 * 茉莉大脑 v2 — 智慧宠物认知状态机（纯 TS，零依赖，可单测）。
 *
 * v2「新维度」升级：
 * - **情境语言**：根据最近事件流生成贴合场景的短评（`remark()`），
 *   让宠物「会说话」——开箱、工具调用、告警、久未互动各有台词。
 * - **昼夜节律**：感知本地时间，早晨精神、深夜困倦，驱动基础唤醒度。
 * - **学习统计**：记录任务完成数 / 观察 token / 工具调用数，随互动成长。
 * - **多心情**：新增 `thinking`（规划中托腮）、`laughing`（被逗乐）。
 *
 * 输入源（webui RPC 总线 + 本地指针/时钟）：
 *   text_delta   → attentive（流式回答中，专注注视）
 *   tool_start   → working （工具执行，尾巴高频摆动）
 *   tool_result  → 回落至 attentive
 *   warning      → alert   （耳朵压平，全身紧绷）
 *   done         → celebrate（成功完成，跳跃撒花 + 经验值）
 *   plan         → thinking（规划阶段，托腮沉思）
 *   pointer      → curious （视线追踪鼠标）
 *   tick(dt)     → 唤醒度自然衰减 + 昼夜调制；持续空闲 → sleepy
 *
 * 输出 BrainFrame：mood / arousal(0..1) / gaze(x,y) / hearts / level / xp / stats。
 */

export type PetMood =
  | 'idle'
  | 'curious'
  | 'attentive'
  | 'working'
  | 'celebrate'
  | 'alert'
  | 'sleepy'
  | 'thinking'
  | 'laughing'

export interface BrainFrame {
  mood: PetMood
  /** 0..1 — 驱动尾频/耳角/呼吸幅度/瞳孔缩放 */
  arousal: number
  /** -1..1 视线目标（相对屏幕中心） */
  gazeX: number
  gazeY: number
  /** celebrate 期间为 true（渲染层播放跳跃+爱心） */
  hearts: boolean
  /** 智慧等级（经验驱动） */
  level: number
  xp: number
  /** 成长统计（v2） */
  stats: {
    tasksDone: number
    toolsUsed: number
    tokensSeen: number
  }
}

export interface PetRemark {
  text: string
  /** 情绪表情前缀（渲染层可加气泡样式） */
  emoji: string
}

const STORAGE_KEY = 'pet.brain.v2'

const AROUSET_TARGET: Record<PetMood, number> = {
  idle: 0.35,
  curious: 0.55,
  attentive: 0.78,
  working: 0.92,
  celebrate: 1,
  alert: 1,
  sleepy: 0.12,
  thinking: 0.62,
  laughing: 0.95,
}

/** 每秒唤醒衰减速率（朝当前 mood 目标值收敛） */
const DECAY_PER_SEC = 0.55
/** 连续空闲多少秒后进入睡眠 */
const SLEEP_AFTER_S = 45
/** celebrate 持续秒数 */
const CELEBRATE_S = 2.6

export interface PetBrainOptions {
  /** 每次庆祝获得的经验（默认 4） */
  xpPerCelebrate?: number
}

export class PetBrain {
  mood: PetMood = 'idle'
  arousal = 0.35
  gazeX = 0
  gazeY = 0
  xp = 0
  stats = { tasksDone: 0, toolsUsed: 0, tokensSeen: 0 }

  private idleSeconds = 0
  private celebrateLeft = 0
  private readonly xpPerCelebrate: number
  /** 最近一次提醒时间（防刷屏） */
  private lastRemarkAt = 0
  /** 最近事件循环，供情境语言引用 */
  private recentEvents: string[] = []

  constructor(options: PetBrainOptions = {}) {
    this.xpPerCelebrate = options.xpPerCelebrate ?? 4
    this.restore()
  }

  // ------------------------------------------------------------------ input
  feed(event: 'text_delta' | 'tool_start' | 'tool_result' | 'warning' | 'done' | 'queued' | 'plan' | 'laugh'): void {
    this.idleSeconds = 0
    this.recentEvents.push(event)
    if (this.recentEvents.length > 6) this.recentEvents.shift()
    switch (event) {
      case 'text_delta':
        this.mood = 'attentive'
        this.stats.tokensSeen += 12 // 估算
        break
      case 'tool_start':
        this.mood = 'working'
        this.stats.toolsUsed += 1
        break
      case 'tool_result':
        if (this.mood === 'working') this.mood = 'attentive'
        break
      case 'warning':
        this.mood = 'alert'
        break
      case 'queued':
        this.mood = 'curious'
        break
      case 'plan':
        this.mood = 'thinking'
        break
      case 'laugh':
        this.mood = 'laughing'
        break
      case 'done':
        this.mood = 'celebrate'
        this.celebrateLeft = CELEBRATE_S
        this.stats.tasksDone += 1
        this.xp += this.xpPerCelebrate
        this.persist()
        break
    }
    this.arousal = Math.max(this.arousal, AROUSET_TARGET[this.mood])
  }

  /** 指针移动 → 视线追踪 + 好奇（不覆盖更高唤醒的状态） */
  pointer(nx: number, ny: number): void {
    this.gazeX = clamp(nx, -1, 1)
    this.gazeY = clamp(ny, -1, 1)
    if (this.mood === 'idle' || this.mood === 'sleepy') {
      this.mood = 'curious'
      this.idleSeconds = 0
    }
  }

  /** 用户抚摸（双击）→ 立即庆祝 + 经验 */
  petted(): void {
    this.feed('done')
  }

  // ------------------------------------------------------------------ clock
  tick(dt: number): BrainFrame {
    const d = Math.min(Math.max(dt, 0), 0.25) // 防后台标签页巨步
    this.idleSeconds += d

    if (this.celebrateLeft > 0) {
      this.celebrateLeft -= d
      if (this.celebrateLeft <= 0 && this.mood === 'celebrate') this.mood = 'idle'
    }

    if (this.idleSeconds > SLEEP_AFTER_S && this.celebrateLeft <= 0) {
      this.mood = 'sleepy'
    } else if (
      (this.mood === 'attentive' || this.mood === 'curious' || this.mood === 'alert') &&
      this.idleSeconds > 6
    ) {
      this.mood = 'idle'
    }

    const target = AROUSET_TARGET[this.mood]
    const rate = target < this.arousal ? DECAY_PER_SEC : DECAY_PER_SEC * 2.2
    const delta = target - this.arousal
    this.arousal += clamp(delta, -rate * d, rate * d)

    return this.frame()
  }

  frame(): BrainFrame {
    return {
      mood: this.mood,
      arousal: round2(this.arousal),
      gazeX: round2(this.gazeX),
      gazeY: round2(this.gazeY),
      hearts: this.celebrateLeft > 0,
      level: this.level(),
      xp: this.xp,
      stats: { ...this.stats },
    }
  }

  level(): number {
    return 1 + Math.floor(this.xp / 50)
  }

  /**
   * 情境语言 v2：基于当前心情 + 最近事件 + 昼夜节律，生成一句符合场景的话。
   * 每 8 秒最多一次，避免「话痨」。渲染层据此显示气泡。
   */
  remark(now = Date.now()): PetRemark | null {
    if (now - this.lastRemarkAt < 8000) return null
    const hour = new Date(now).getHours()
    const isNight = hour >= 23 || hour < 5
    const isMorning = hour >= 6 && hour < 10

    const text = (() => {
      switch (this.mood) {
        case 'working':
          return pick([
            '正在处理，专注中…',
            '工具在响，交给我吧',
            '让我看看这一步怎么做',
          ])
        case 'thinking':
          return pick([
            '让我想想最优方案…',
            '规划中，先拆解再执行',
            '嗯…这样更稳妥',
          ])
        case 'alert':
          return pick([
            '有告警，需要留意一下',
            '这个信号有点异常',
          ])
        case 'celebrate':
          return pick([
            '搞定！我攒到经验了',
            '顺利完成，好有成就感',
            'Nice，任务闭环！',
          ])
        case 'sleepy':
          if (isNight) return '夜深了，陪我一起休息吧…'
          return pick(['有点困了…', '眯一会儿…'])
        case 'curious':
          return pick(['你在看什么呀？', '那边有什么有趣的吗？', '我也想看！'])
        case 'attentive':
          return pick(['认真听着呢', '正在消化这些内容…', '继续说，我在记'])
        case 'laughing':
          return '哈哈，被逗笑了！'
        default:
          if (isMorning) return '早呀，今天元气满满！'
          if (isNight) return '晚上好，我在安静陪着你'
          return pick(['我在这里', '随时待命', '等你回来～'])
      }
    })()

    this.lastRemarkAt = now
    const emoji = moodEmoji(this.mood)
    return { text, emoji }
  }

  // ------------------------------------------------------------ persistence
  private restore(): void {
    try {
      const raw = localStorage.getItem(STORAGE_KEY)
      if (!raw) return
      const parsed = JSON.parse(raw) as { xp?: number; stats?: Partial<PetBrain['stats']> }
      if (typeof parsed.xp === 'number' && parsed.xp >= 0) this.xp = Math.floor(parsed.xp)
      if (parsed.stats && typeof parsed.stats === 'object') {
        this.stats = {
          tasksDone: num(parsed.stats.tasksDone),
          toolsUsed: num(parsed.stats.toolsUsed),
          tokensSeen: num(parsed.stats.tokensSeen),
        }
      }
    } catch {
      /* 损坏数据按全新宠物处理 */
    }
  }

  private persist(): void {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify({ xp: this.xp, stats: this.stats }))
    } catch {
      /* 隐私模式下静默跳过 */
    }
  }
}

function moodEmoji(mood: PetMood): string {
  switch (mood) {
    case 'working': return '💭'
    case 'thinking': return '🤔'
    case 'alert': return '⚠️'
    case 'celebrate': return '🎉'
    case 'sleepy': return '💤'
    case 'curious': return '👀'
    case 'laughing': return '😆'
    case 'attentive': return '📖'
    default: return '✨'
  }
}

function pick(arr: string[]): string {
  return arr[Math.floor(Math.random() * arr.length)]
}

function num(v: unknown): number {
  return typeof v === 'number' && Number.isFinite(v) && v >= 0 ? Math.floor(v) : 0
}

function clamp(v: number, lo: number, hi: number): number {
  return Math.min(Math.max(v, lo), hi)
}

function round2(v: number): number {
  return Math.round(v * 100) / 100
}

// ---------------------------------------------------------------------------
// 单例：桌面悬浮宠与宠物页共享同一个大脑（同一份情绪/记忆）
// ---------------------------------------------------------------------------
let singleton: PetBrain | null = null

export function getPetBrain(): PetBrain {
  if (!singleton) singleton = new PetBrain()
  return singleton
}
