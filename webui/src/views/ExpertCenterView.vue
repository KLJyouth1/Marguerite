<template>
  <div class="control-stage expert-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <p class="expert-kicker">Agents · Experts · Swarm</p>
        <h1 class="control-stage__title">智能体与专家中心</h1>
        <p class="control-stage__subtitle">预设智能体 / 专家库（蒸馏行业经验）/ 模式蜂群规格 — 子代理扇出、工具白名单与记忆检索同一编排面。</p>
      </div>
      <div class="control-stage__actions"><button class="btn btn--ghost" @click="loadAll"><Icon name="refresh" :size="16" /><span>刷新</span></button></div>
    </header>

    <section v-if="error" class="expert-error" role="alert">{{ error }}</section>

    <section class="expert-constellation" aria-hidden="true">
      <span v-for="n in expertOrbit" :key="n" class="expert-constellation__node">{{ n }}</span>
    </section>

    <!-- 主智能体（主视觉）：桌面端所有默认会话（agent:main:*）都由它应答 -->
    <section
      v-if="mainAgent"
      class="ma-hero"
      aria-labelledby="ma-hero-name"
    >
      <div class="ma-hero__mark" aria-hidden="true">
        <img
          v-if="avatarOk"
          class="ma-hero__avatar"
          :src="mainAvatar"
          alt=""
          width="56"
          height="56"
          @error="avatarOk = false"
        />
        <template v-else>{{ mainInitial }}</template>
      </div>
      <div class="ma-hero__body">
        <p class="ma-hero__kicker">主智能体 · Main Agent</p>
        <h2 id="ma-hero-name" class="ma-hero__name">{{ mainAgent.name }}</h2>
        <p class="ma-hero__desc">{{ mainAgent.description }}</p>
        <ul v-if="mainAgent.skills && mainAgent.skills.length" class="ma-hero__chips">
          <li v-for="s in mainAgent.skills" :key="s" class="ma-chip">{{ s }}</li>
        </ul>
        <p class="ma-hero__meta">
          <span>id <code>{{ mainAgent.id }}</code></span>
          <span>工具 {{ mainAgent.tools ? mainAgent.tools.length : 0 }}</span>
          <span>技能 {{ mainAgent.skills ? mainAgent.skills.length : 0 }}</span>
          <span>模型 {{ mainAgent.model || '默认路由' }}</span>
          <span :class="['ma-hero__state', mainAgent.enabled === false ? 'is-off' : 'is-on']">
            {{ mainAgent.enabled === false ? '已停用（人格不注入）' : '已启用' }}
          </span>
        </p>
      </div>
      <div class="ma-hero__actions">
        <button class="btn btn--primary" type="button" @click="startMainChat">开始对话</button>
        <button
          class="btn btn--ghost"
          type="button"
          :aria-expanded="personaOpen"
          aria-controls="ma-persona"
          @click="personaOpen = !personaOpen"
        >{{ personaOpen ? '收起人设' : '查看人设' }}</button>
        <button class="btn btn--ghost" type="button" @click="openAgents">智能体管理</button>
      </div>
    </section>

    <!-- 主智能体加载中：骨架，而不是把「还没加载」渲染成「没有智能体」 -->
    <section v-else-if="loading" class="ma-hero ma-hero--skeleton" aria-busy="true" aria-live="polite">
      <div class="ma-hero__mark ma-skeleton" aria-hidden="true"></div>
      <div class="ma-hero__body">
        <p class="ma-skeleton ma-skeleton--line" style="width: 30%"></p>
        <p class="ma-skeleton ma-skeleton--line ma-skeleton--title" style="width: 46%"></p>
        <p class="ma-skeleton ma-skeleton--line" style="width: 82%"></p>
        <p class="ma-skeleton ma-skeleton--line" style="width: 64%"></p>
      </div>
      <span class="ma-sr">正在读取主智能体…</span>
    </section>

    <!-- 主智能体缺失：如实说明，不伪造一个占位身份。
         注意 `!error` 这道门：清单读取失败时 `agents` 也是空数组，
         若不区分，「请求失败」会被渲染成「网关里没有 main」——把异常态说成空态。 -->
    <section v-else-if="!error" class="ma-hero ma-hero--missing" role="status">
      <div class="ma-hero__body">
        <p class="ma-hero__kicker">主智能体 · Main Agent</p>
        <h2 class="ma-hero__name">未找到主智能体（id: main）</h2>
        <p class="ma-hero__desc">
          网关返回的智能体清单里没有 <code>main</code>。默认会话（<code>agent:main:webchat:*</code>）
          仍可对话，但不会注入任何人设。请到「智能体管理」页确认内置智能体是否被停用或清单读取失败。
        </p>
        <p class="ma-hero__meta"><button class="btn btn--ghost btn--sm" type="button" @click="loadAll">重新读取</button></p>
      </div>
    </section>

    <section v-if="mainAgent && personaOpen" id="ma-persona" class="expert-card ma-persona">
      <div class="expert-card__title">主智能体人设（system prompt · 只读）</div>
      <pre class="ma-persona__text">{{ mainAgent.system_prompt || mainAgent.systemPrompt || '（该智能体未设置人设）' }}</pre>
      <p class="expert-note">
        人设来自内置台账 <code>agent/personas/senior-dev-engineer.md</code>，随会话键
        <code>agent:main:*</code> 注入系统提示。内置智能体不可在此编辑；如需定制，
        到「智能体管理」用「自定义」复制一份再改。
      </p>
    </section>

    <!-- 模式与蜂群 -->
    <section v-if="spec" class="expert-card">
      <div class="expert-card__title">模式与蜂群（mode.spec）</div>
      <div class="expert-pills">
        <span class="expert-pill expert-pill--accent">当前模式：{{ spec.label }}</span>
        <span class="expert-pill">风险等级 {{ spec.risk_level }}</span>
        <span class="expert-pill">工具白名单 {{ spec.allowed_tools }} / 黑名单 {{ spec.denied_tools }}</span>
      </div>
      <div v-if="spec.bloom" class="expert-pills">
        <span class="expert-pill">蜂群分片上限（数量上限）{{ spec.bloom.max_swarm_shards }}</span>
        <span class="expert-pill">子专家上限 {{ spec.bloom.max_sub_experts }}</span>
        <span class="expert-pill">推理深度 {{ spec.bloom.thinking_depth }}</span>
        <span class="expert-pill">验证轮数 {{ spec.bloom.verification_passes }}</span>
        <span class="expert-pill">思考强度 {{ spec.bloom.reasoning_effort }}</span>
        <span class="expert-pill">快照安全网 {{ spec.bloom.snapshot_enabled ? '开' : '关' }}</span>
        <span class="expert-pill">自主闭环预算 {{ spec.bloom.budget_tokens }} token / {{ spec.bloom.max_iterations }} 轮</span>
      </div>
      <p class="expert-note">蜂群数量上限由内置模式规格（bloom.yaml / omni_transcend.yaml）决定；智能体、智能体团队、专家团队在 Bloom / OmniTranscend 模式下均可按此上限启动蜂群并行攻坚。</p>
    </section>
    <!-- 模式规格缺失：此前整张卡静默消失，用户看到的是「页面少了块东西」而不是失败 -->
    <section v-else class="expert-card">
      <div class="expert-card__title">模式与蜂群（mode.spec）</div>
      <p class="expert-note expert-note--err">
        模式规格未读取到（mode.spec 失败或网关未就绪）——本卡不渲染任何猜测值。
      </p>
    </section>

    <!-- 蜂群控制（数量上限设置） -->
    <section class="expert-card">
      <div class="expert-card__title">蜂群控制 · 数量上限设置（mode.swarm.get / set）</div>
      <template v-if="swarm">
        <div class="expert-form">
          <label class="expert-field">
            <span>蜂群分片上限（0 = 内置默认；硬上限 {{ swarm.maxShardsCap }}）</span>
            <input v-model.number="swarmShards" type="number" min="0" :max="swarm.maxShardsCap" class="expert-input" />
          </label>
          <label class="expert-field">
            <span>子专家上限（0 = 内置默认；硬上限 {{ swarm.maxSubExpertsCap }}）</span>
            <input v-model.number="swarmSubExperts" type="number" min="0" :max="swarm.maxSubExpertsCap" class="expert-input" />
          </label>
          <button class="btn btn--primary" :disabled="swarmBusy" @click="saveSwarm">保存上限</button>
          <button class="btn btn--ghost" :disabled="swarmBusy" @click="resetSwarm">恢复内置默认</button>
        </div>
        <div class="expert-pills expert-pills--swarm">
          <span class="expert-pill">生效分片上限 {{ swarm.effectiveShards }}</span>
          <span class="expert-pill">生效子专家上限 {{ swarm.effectiveSubExperts }}</span>
          <span class="expert-pill">已配置 {{ swarm.configuredShards ?? '内置默认' }} / {{ swarm.configuredSubExperts ?? '内置默认' }}</span>
        </div>
        <p class="expert-note">智能体 / 团队 / 专家启动蜂群时按此上限扇出（orchestration run_bloom_swarm 实时读取）；持久化 data_dir/swarm_config.json，重启不丢。</p>
      </template>
      <p v-else class="expert-note expert-note--err">
        上限未读取到（mode.swarm.get 失败或网关未就绪）——此处<b>不显示任何猜测值</b>，避免把
        「未知」伪装成「内置默认 11 / 5」。<button class="btn btn--ghost btn--sm" type="button" @click="loadSwarm">重试</button>
      </p>
    </section>

    <!-- 团队 / 蜂群一键启动 -->
    <section class="expert-card">
      <div class="expert-card__title">团队 / 蜂群一键启动（team.run · 真实并行 spawn）</div>
      <div class="expert-form">
        <input v-model="teamGoal" class="expert-input expert-input--wide" placeholder="目标描述（如：为登录功能设计接口契约、实现代码、编写单测）" @keyup.enter="launchTeam" />
        <button class="btn btn--primary" :disabled="teamBusy || !teamGoal.trim()" @click="launchTeam">{{ teamBusy ? '团队执行中…' : '启动专家团队' }}</button>
      </div>
      <p class="expert-note">自动角色检测 → 专家成员并行 spawn → lead 综合。未配置 API Key 时成员真实启动但 LLM 调用失败（如实回报）。蜂群（Bloom 双领队）在 Bloom / OmniTranscend 模式任务分析时自动扇出，数量上限见「蜂群控制」卡。</p>
      <div v-if="teamResult" class="expert-team">
        <div class="expert-team__head">策略 {{ teamResult.strategy }} · 成员产出 {{ teamResult.members }} 段</div>
        <ul v-if="teamResult.summaries && teamResult.summaries.length">
          <li v-for="(s, i) in teamResult.summaries" :key="i">{{ s }}</li>
        </ul>
        <pre v-if="teamResult.lead_reply">{{ teamResult.lead_reply }}</pre>
      </div>
      <div v-if="teamError" class="expert-note expert-note--err">{{ teamError }}</div>
    </section>

    <!-- 蜂群后台化 -->
    <section class="expert-card">
      <div class="expert-card__title">蜂群后台化（team.run.background · long_running 台账）</div>
      <div class="expert-form">
        <input v-model="bgGoal" class="expert-input expert-input--wide" placeholder="后台蜂群目标…（立即返回 taskId，执行不阻塞）" @keyup.enter="launchBackgroundTeam" />
        <button class="btn btn--ghost" :disabled="bgBusy || !bgGoal.trim()" @click="launchBackgroundTeam">启动后台蜂群</button>
      </div>
      <p v-if="bgNote" class="expert-note">{{ bgNote }}</p>
      <div v-if="bgTask" class="expert-pills expert-pills--swarm">
        <span class="expert-pill">任务 {{ bgTask.taskId }}</span>
        <span class="expert-pill">状态 {{ bgTask.status }}</span>
        <span class="expert-pill">重试 {{ bgTask.retryCount }}/{{ bgTask.maxRetries }}</span>
        <span class="expert-pill">检查点 {{ bgTask.checkpoints?.length ?? 0 }}</span>
      </div>
      <p class="expert-note">后台蜂群注册进 long_running 台账（心跳 15s / 检查点 / 完成态），引擎健康页「任务执行质量」卡同步可见；lead 综合与成员产出写入检查点。</p>
    </section>

    <!-- 专家团队持久化组合 -->
    <section class="expert-card">
      <div class="expert-card__title">专家团队持久化组合（team.compositions.*）</div>
      <div class="expert-form">
        <input v-model="compName" class="expert-input" placeholder="组合名" />
        <input v-model="compGoal" class="expert-input expert-input--wide" placeholder="目标描述" />
        <textarea v-model="compMembers" class="expert-textarea" placeholder="成员清单（每行：角色：简述）&#10;架构设计：接口契约&#10;编码实现：核心逻辑" />
        <button class="btn btn--primary" :disabled="compBusy || !compName.trim()" @click="saveComp">保存组合</button>
      </div>
      <div v-for="c in comps" :key="c.id" class="expert-comp">
        <b>{{ c.name }}</b>
        <span class="expert-item__desc">{{ c.goal }}</span>
        <span class="expert-item__desc">{{ (c.members ?? []).map(m => m.role).join(' / ') }}</span>
        <span class="expert-comp__actions">
          <button class="btn btn--sm btn--ghost" :disabled="compBusy" @click="loadComp(c)">载入</button>
          <button class="btn btn--sm btn--primary" :disabled="compBusy" @click="rerunComp(c)">后台重跑</button>
          <button class="btn btn--sm btn--ghost" :disabled="compBusy" @click="removeComp(c.name)">删除</button>
        </span>
      </div>
      <p v-if="comps.length === 0" class="expert-note">暂无已保存组合 —— 填写上方表单保存；重跑会把成员清单作为团队构成注入目标（参与角色检测）。</p>
    </section>

    <!-- 用户行为蒸馏（用户画像，不是主智能体画像） -->
    <section class="expert-card">
      <div class="expert-card__title">用户行为蒸馏 → 适配主智能体</div>
      <div v-if="behavior" class="expert-pills">
        <span class="expert-pill">行为事件 {{ behavior.events ?? 0 }}</span>
        <span class="expert-pill">对话轮次 {{ behavior.turns ?? 0 }}</span>
        <span class="expert-pill">工具 Top：<span v-for="(t, i) in (behavior.tool_uses ?? [])" :key="i">{{ t.tool }}×{{ t.count }} </span></span>
      </div>
      <p v-else class="expert-note expert-note--err">
        行为汇总未读取到（memory.behavior.summary 失败或网关未就绪）——不显示任何 0 值假象。
      </p>
      <div v-if="profile" class="expert-pills expert-pills--swarm">
        <span class="expert-pill expert-pill--accent">用户画像：{{ profile.role }} · {{ profile.tone }}</span>
        <span class="expert-pill">专长：{{ profile.expertise }}</span>
        <span class="expert-pill">来源 {{ profile.source_count }} 条上下文</span>
        <span class="expert-pill">{{ profileInjected ? '已注入主智能体' : '未注入（样本不足）' }}</span>
      </div>
      <p class="expert-note">
        每轮对话经 record_behavior 落账（B3 行为捕获，memory.behavior.summary 聚合）；画像是
        <b>用户的</b>画像，由 agent.profile.auto 从最近上下文确定性抽取（不烧 token）。
        命中高置信度（角色非中性且样本 ≥ {{ profileMinSources }} 条）时，以一行软提示注入主智能体
        用于调整术语深度与举例；否则<b>不注入</b>——宁可不猜，也不误导。
      </p>
    </section>

    <!-- 预设智能体 -->
    <section class="expert-card">
      <div class="expert-card__title">预设智能体（agents.list，内置在前）</div>
      <div v-if="agents.length" class="expert-grid">
        <div v-for="a in agents" :key="a.id" class="expert-item" :class="{ 'is-main': a.id === 'main' }">
          <b>{{ a.name || a.id }}</b>
          <code class="expert-item__id">{{ a.id }}</code>
          <span class="expert-item__meta">
            <i v-if="a.is_builtin" class="expert-tag">内置</i>
            <i v-if="a.id === 'main'" class="expert-tag">主智能体</i>
            <i v-if="a.enabled === false" class="expert-tag expert-tag--dim">停用</i>
          </span>
          <span class="expert-item__desc">{{ a.description || '（无描述）' }}</span>
          <span class="expert-item__desc">{{ a.model || '（默认模型路由）' }} · 工具 {{ a.tools ? a.tools.length : 0 }}</span>
        </div>
      </div>
      <div v-else-if="loading" class="expert-pills">
        <span class="expert-pill ma-skeleton ma-skeleton--line" style="width: 180px"></span>
        <span class="expert-pill ma-skeleton ma-skeleton--line" style="width: 140px"></span>
      </div>
      <p v-else class="expert-note">
        网关返回的智能体清单为空 —— 内置智能体由 <code>seed_builtins()</code> 在首次启动时写入
        <code>data_dir/agents.json</code>，为空说明台账未初始化或读取失败。可到「智能体管理」页刷新查看。
      </p>
    </section>

    <!-- 预设专家库（可在对话框 @ 引用） -->
    <section class="expert-card">
      <div class="expert-card__title">专家团 / 预设专家库（experts.list · 可在对话框 @ 引用）</div>
      <p class="expert-note">
        在对话框输入 <b>@</b> 即可从下方专家与预设智能体中选取引用；此处「在对话框引用」会复制
        <code>@名称</code> 到剪贴板，粘贴进输入框即可（引用为消息文本标注，不改写发送负载）。
      </p>
      <p v-if="citeNote" class="expert-note expert-note--ok">{{ citeNote }}</p>
      <div v-if="experts.length" class="expert-grid">
        <article v-for="e in experts" :key="e.id" class="expert-item">
          <b>{{ e.label }}</b>
          <span class="expert-item__desc">{{ e.lineage }}</span>
          <span class="expert-item__desc">{{ e.mindset }}</span>
          <span class="expert-item__desc">专长：{{ joinList(e.expertise) }}</span>
          <span class="expert-item__desc">专属技能：{{ joinList(e.skills) }}</span>
          <span class="expert-item__playbook">剧本 {{ (e.playbook || []).length }} 步</span>
          <button class="btn btn--sm btn--ghost expert-item__cite" @click="citeExpert(e)">在对话框引用 @</button>
        </article>
      </div>
      <div v-else-if="loading" class="expert-pills">
        <span class="expert-pill ma-skeleton ma-skeleton--line" style="width: 200px"></span>
        <span class="expert-pill ma-skeleton ma-skeleton--line" style="width: 160px"></span>
      </div>
      <p v-else class="expert-note">
        网关返回的专家库为空（<code>experts.list</code> 正常但列表为 0）—— 检查 <code>agent::expertise</code>
        的内置专家库是否被裁空；若同时伴随上方红色错误，则属于网关未就绪。
      </p>
    </section>

    <!-- 团队 / 蜂群 / 第三方说明 -->
    <section class="expert-card">
      <div class="expert-card__title">智能体团队 / 专家团队 / 蜂群 / 第三方生态</div>
      <ul class="expert-list">
        <li>智能体团队：core/agent/team.rs 已提供 Team/Mailbox/依赖拓扑（真实核心能力）；团队编排 UI 逐步接线（诚实状态）。</li>
        <li>蜂群：Bloom / OmniTranscend 模式内建双领队蜂群，数量上限见上「模式与蜂群」卡（max_swarm_shards）。</li>
        <li>三种并行协调（智能体 / 团队 / 专家）：dispatch 扇出 + subagent trellis 事件驱动（core/orchestration 已接线）。</li>
        <li>第三方专家与专家团队：market.import 可导入第三方插件清单；专家档案经 experts.list 展示（蒸馏式预设）。</li>
      </ul>
    </section>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'
// 主智能体头像来自专家包 `senior-dev-engineer/avatars/expert.png`（10 KB）。
// 载入失败时自动退回首字标记，不出现破图。
import mainAvatar from '@/assets/agents/senior-dev-engineer.png'

const rpc = useRpcStore()
const router = useRouter()
const expertOrbit = ['Lead', 'Architect', 'Builder', 'Reviewer', 'Memory', 'Tools', 'Knowledge']

/**
 * 专家库条目。`expertise` / `skills` 在网关侧是**数组**
 * （`gateway/team.rs::experts_list` 直接序列化 `&[&str]`），
 * 此前这里声明成 `string` 属于契约谎言——运行时靠 JS 隐式 `Array#toString`
 * 才没炸，一旦改用 `.length` / `.map` 就会得到错值。
 */
interface Expert {
  id: string
  label: string
  lineage: string
  mindset: string
  expertise?: string[]
  skills?: string[]
  playbook?: string[]
}
interface AgentItem {
  id: string
  name?: string
  description?: string
  is_builtin?: boolean
  enabled?: boolean
  model?: string
  tools?: string[]
  skills?: string[]
  system_prompt?: string
  systemPrompt?: string
}
interface ModeSpec { kind: string; label: string; risk_level: string; allowed_tools: number; denied_tools: number; bloom?: { max_swarm_shards: number; max_sub_experts: number; thinking_depth: number; verification_passes: number; reasoning_effort: string; snapshot_enabled: boolean; budget_tokens: number; max_iterations: number } | null }

const experts = ref<Expert[]>([])
const agents = ref<AgentItem[]>([])
const spec = ref<ModeSpec | null>(null)
const error = ref('')
const citeNote = ref('')
/** 首屏加载中：把「还没回来」与「真的为空」分开渲染，避免空态闪现。 */
const loading = ref(true)
const personaOpen = ref(false)
/** 头像加载失败 → 退回首字标记（不渲染破图）。 */
const avatarOk = ref(true)

/** 主智能体 = 默认会话（`agent:main:*`）的应答者。 */
const mainAgent = computed<AgentItem | null>(
  () => agents.value.find(a => a.id === 'main') ?? null,
)
const mainInitial = computed(() => (mainAgent.value?.name || 'M').trim().slice(0, 1) || 'M')

/** 用户画像注入门槛：与后端 `agent::profile::USER_PROFILE_MIN_SOURCES` 对齐。 */
const profileMinSources = 20
const profileInjected = computed(() => {
  const p = profile.value
  if (!p) return false
  const role = (p.role || '').trim()
  const neutral = !role || role === '通用助手' || role === '通用用户'
  return (p.source_count ?? 0) >= profileMinSources && !neutral
})

/** 数组/字符串统一渲染（后端既有数组也有单串的历史契约）。 */
function joinList(v?: string[] | string): string {
  if (Array.isArray(v)) return v.length ? v.join('、') : '—'
  return v || '—'
}

/**
 * 开始与主智能体对话：进入**草稿路由**（`/chat/new`）并带上 agent=main，
 * 而不是跳到固定会话键 `agent:main:webchat:default`。
 *
 * 理由（踩过的坑）：落地页/建议 chips 只在草稿路由渲染（`isDraftRoute()` 门），
 * 固定会话键要么带着旧历史直接进对话，要么渲染成空会话——用户点「开始对话」
 * 期待的是「一段新的对话」，不是「回到上次那个会话」。
 */
function startMainChat() {
  void router.push({ path: '/chat/new', query: { agent: 'main' } })
}

function openAgents() {
  void router.push('/agents')
}

/** 复制 `@名称` 到剪贴板，供用户在对话框输入框中引用该专家。 */
async function citeExpert(e: Expert) {
  const ref_ = `@${e.label}`
  try {
    await navigator.clipboard.writeText(ref_)
    citeNote.value = `已复制「${ref_}」——粘贴进对话框输入框即可引用`
  } catch {
    citeNote.value = `引用写法：「${ref_}」——在对话框输入框粘贴即可`
  }
  window.setTimeout(() => { citeNote.value = '' }, 5000)
}

async function loadAll() {
  error.value = ''
  loading.value = true
  try {
    // 冷加载竞态守卫：WS 仍处于 connecting 时先等连接就绪，避免把「未就绪」
    // 当成加载失败，把 `not connected (state: connecting)` 裸串渲染给用户。
    if (!rpc.isConnected) await rpc.waitForConnection(10_000)
    const [e, a, s] = await Promise.all([
      rpc.call<{ experts?: Expert[] }>('experts.list', {}),
      rpc.call<{ agents?: AgentItem[] }>('agents.list', {}),
      rpc.call<ModeSpec>('mode.spec', {}),
    ])
    experts.value = e?.experts ?? []
    agents.value = a?.agents ?? []
    spec.value = s ?? null
  } catch (err) {
    error.value = `加载失败: ${err instanceof Error ? err.message : String(err)}`
  } finally {
    loading.value = false
  }
  // 次要数据源独立加载：任一失败只影响它自己那张卡，且失败会如实呈现（不再静默吞掉）。
  void loadSwarm()
  void loadBehavior()
  void loadComps()
}

interface SwarmCfg { configuredShards?: number | null; configuredSubExperts?: number | null; effectiveShards: number; effectiveSubExperts: number; maxShardsCap: number; maxSubExpertsCap: number }
/**
 * `null` = 尚未读到（未加载 / 读取失败）。
 * 此前默认值是硬编码的 `{ effectiveShards: 11, effectiveSubExperts: 5 }`，
 * 于是 RPC 一失败，界面就把「未知」显示成「内置默认 11 / 5」——纯属编造。
 */
const swarm = ref<SwarmCfg | null>(null)
const swarmShards = ref(0)
const swarmSubExperts = ref(0)
const swarmBusy = ref(false)

async function loadSwarm() {
  try {
    const s = await rpc.call<SwarmCfg>('mode.swarm.get', {})
    // 契约校验：缺关键字段时视为读取失败，而不是把 undefined 渲染成空白
    if (!s || typeof s.effectiveShards !== 'number') throw new Error('mode.swarm.get 返回不完整')
    swarm.value = s
    swarmShards.value = s.configuredShards ?? 0
    swarmSubExperts.value = s.configuredSubExperts ?? 0
  } catch {
    swarm.value = null
  }
}

async function saveSwarm() {
  if (!swarm.value) return
  swarmBusy.value = true
  try {
    const s = await rpc.call<SwarmCfg>('mode.swarm.set', { maxSwarmShards: swarmShards.value, maxSubExperts: swarmSubExperts.value })
    if (!s || typeof s.effectiveShards !== 'number') throw new Error('mode.swarm.set 返回不完整')
    swarm.value = s
    swarmShards.value = s.configuredShards ?? 0
    swarmSubExperts.value = s.configuredSubExperts ?? 0
  } catch {
    swarm.value = null
  } finally { swarmBusy.value = false }
}

async function resetSwarm() {
  swarmShards.value = 0
  swarmSubExperts.value = 0
  await saveSwarm()
}

interface TeamResult { ok: boolean; strategy?: string; members?: number; summaries?: string[]; lead_reply?: string; error?: string }
interface BehaviorSummary { events?: number; turns?: number; tool_uses?: Array<{ tool: string; count: number }> }
interface AutoProfile { role?: string; tone?: string; expertise?: string; source_count?: number }
const teamGoal = ref('')
const teamBusy = ref(false)
const teamResult = ref<TeamResult | null>(null)
const teamError = ref('')
const behavior = ref<BehaviorSummary | null>(null)
const profile = ref<AutoProfile | null>(null)

async function launchTeam() {
  const goal = teamGoal.value.trim()
  if (!goal) return
  teamBusy.value = true
  teamError.value = ''
  teamResult.value = null
  try {
    const r = await rpc.call<TeamResult>('team.run', { goal })
    if (r.ok) teamResult.value = r
    else teamError.value = r.error ?? '团队执行失败'
  } catch (e) { teamError.value = e instanceof Error ? e.message : String(e) } finally { teamBusy.value = false }
}

async function loadBehavior() {
  try {
    behavior.value = (await rpc.call<BehaviorSummary>('memory.behavior.summary', {})) ?? null
  } catch {
    // 失败保持 null：模板据此渲染「未读取到」，而不是 0 值假象
    behavior.value = null
  }
  try {
    profile.value = (await rpc.call<AutoProfile>('agent.profile.auto', {})) ?? null
  } catch {
    profile.value = null
  }
}

interface BgTask { taskId: string; status: string; retryCount: number; maxRetries: number; checkpoints?: unknown[] }
interface Composition { id: string; name: string; goal: string; members: Array<{ role: string; brief: string }> }
const bgGoal = ref('')
const bgBusy = ref(false)
const bgNote = ref('')
const bgTask = ref<BgTask | null>(null)
let bgTimer: ReturnType<typeof setInterval> | null = null
const compName = ref('')
const compGoal = ref('')
const compMembers = ref('')
const comps = ref<Composition[]>([])
const compBusy = ref(false)

async function pollBgTask(taskId: string) {
  try {
    const r = await rpc.call<{ tasks: BgTask[] }>('long_running.list', {})
    const t = (r.tasks ?? []).find(x => x.taskId === taskId)
    if (t) {
      bgTask.value = t
      if (t.status === 'completed' || t.status === 'failed' || t.status === 'interrupted') {
        if (bgTimer) { clearInterval(bgTimer); bgTimer = null }
      }
    }
  } catch { /* 静默 */ }
}

async function launchBackgroundTeam() {
  const goal = bgGoal.value.trim()
  if (!goal) return
  bgBusy.value = true
  bgNote.value = ''
  try {
    const r = await rpc.call<{ ok: boolean; taskId?: string; message?: string; error?: string }>('team.run.background', { goal })
    if (r.ok && r.taskId) {
      bgNote.value = r.message ?? '后台蜂群已启动'
      if (bgTimer) clearInterval(bgTimer)
      bgTimer = setInterval(() => void pollBgTask(r.taskId as string), 5000)
      void pollBgTask(r.taskId)
    } else { bgNote.value = r.error ?? '启动失败' }
  } catch (e) { bgNote.value = e instanceof Error ? e.message : String(e) } finally { bgBusy.value = false }
}

async function loadComps() {
  try {
    const r = await rpc.call<{ ok: boolean; items: Composition[] }>('team.compositions.list', {})
    comps.value = r.items ?? []
  } catch { /* 静默 */ }
}

function parseMembers(text: string): Array<{ role: string; brief: string }> {
  return text.split('\n')
    .map(l => {
      const t = l.trim()
      if (!t) return { role: '', brief: '' }
      const sep = t.indexOf('：') >= 0 ? t.indexOf('：') : t.indexOf(':')
      if (sep < 0) return { role: t, brief: '' }
      return { role: t.slice(0, sep).trim(), brief: t.slice(sep + 1).trim() }
    })
    .filter(m => m.role)
}

async function saveComp() {
  const name = compName.value.trim()
  if (!name) return
  compBusy.value = true
  try {
    const r = await rpc.call<{ ok: boolean; items: Composition[]; error?: string }>('team.compositions.save', { name, goal: compGoal.value.trim(), members: parseMembers(compMembers.value) })
    comps.value = r.items ?? []
    compName.value = ''; compGoal.value = ''; compMembers.value = ''
  } catch { /* 静默 */ } finally { compBusy.value = false }
}

async function removeComp(name: string) {
  compBusy.value = true
  try {
    const r = await rpc.call<{ ok: boolean; items: Composition[] }>('team.compositions.remove', { name })
    comps.value = r.items ?? []
  } catch { /* 静默 */ } finally { compBusy.value = false }
}

function loadComp(c: Composition) {
  compName.value = c.name
  compGoal.value = c.goal
  compMembers.value = (c.members ?? []).map(m => m.brief ? m.role + '：' + m.brief : m.role).join('\n')
}

async function rerunComp(c: Composition) {
  const roster = (c.members ?? []).map(m => m.brief ? m.role + '：' + m.brief : m.role).join('；')
  bgGoal.value = (roster ? '团队构成：' + roster + '。目标：' : '目标：') + c.goal
  await launchBackgroundTeam()
}

// P3 打通：模型/路由变更（本地部署、档案激活、路由保存）与编排 agent 事件
// （orchestrate.run 统一发 started/stopped）→ 专家中心被动刷新，无需手动点刷新。
const rpcUnsubs: Array<() => void> = []
rpcUnsubs.push(
  rpc.on('models.changed', () => void loadAll()),
  rpc.on('agent.started', () => void loadAll()),
  rpc.on('agent.stopped', () => void loadAll()),
)

onMounted(loadAll)
onUnmounted(() => {
  for (const unsub of rpcUnsubs) unsub()
  rpcUnsubs.length = 0
  if (bgTimer) clearInterval(bgTimer)
})
</script>

<style scoped>
.expert-stage { padding: 24px 28px; max-width: 1180px; margin: 0 auto; }
.expert-kicker {
  margin: 0 0 4px;
  font-family: var(--font-mono);
  font-size: 10px;
  letter-spacing: 0.22em;
  text-transform: uppercase;
  color: var(--accent);
}
/* ── 主智能体主视觉 ───────────────────────────────────────────── */
.ma-hero {
  position: relative;
  display: flex;
  gap: 18px;
  align-items: flex-start;
  margin-top: 18px;
  padding: 20px;
  border: 1px solid color-mix(in srgb, var(--accent) 32%, var(--hairline));
  border-radius: var(--radius-card);
  background:
    radial-gradient(120% 140% at 0% 0%, color-mix(in srgb, var(--accent) 14%, transparent), transparent 58%),
    var(--card);
}
.ma-hero__mark {
  flex: none;
  width: 56px;
  height: 56px;
  display: grid;
  place-items: center;
  border-radius: var(--radius-md);
  overflow: hidden;
  font-family: var(--font-display);
  font-size: 26px;
  font-weight: 700;
  color: color-mix(in srgb, var(--accent) 85%, var(--text));
  background: color-mix(in srgb, var(--accent) 16%, var(--card));
  border: 1px solid color-mix(in srgb, var(--accent) 40%, transparent);
}
.ma-hero__avatar {
  width: 100%;
  height: 100%;
  object-fit: cover;
  border-radius: inherit;
  display: block;
}
.ma-hero__body {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.ma-hero__kicker {
  margin: 0;
  font-family: var(--font-mono);
  font-size: 10px;
  letter-spacing: 0.2em;
  text-transform: uppercase;
  color: var(--accent);
}
.ma-hero__name {
  margin: 0;
  font-family: var(--font-display);
  font-size: clamp(1.15rem, 0.9rem + 0.7vw, 1.5rem);
  font-weight: 700;
  color: var(--text);
}
.ma-hero__desc {
  margin: 0;
  font-size: var(--fs-sm);
  line-height: 1.6;
  color: var(--text-dim);
  max-width: 62ch;
}
.ma-hero__chips {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin: 2px 0 0;
  padding: 0;
  list-style: none;
}
.ma-chip {
  padding: 3px 9px;
  border-radius: var(--radius-pill);
  font-size: 11px;
  color: var(--text-dim);
  background: color-mix(in srgb, var(--text) 7%, transparent);
  border: 1px solid var(--hairline);
}
.ma-hero__meta {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
  margin: 4px 0 0;
  font-size: 11px;
  color: var(--text-muted);
}
.ma-hero__meta code {
  font-family: var(--font-mono);
  font-size: 11px;
  color: var(--text-dim);
}
.ma-hero__state.is-on { color: var(--ok, var(--accent)); }
.ma-hero__state.is-off { color: var(--danger); }
.ma-hero__actions {
  flex: none;
  display: flex;
  flex-direction: column;
  gap: 8px;
  min-width: 132px;
}
.ma-hero--missing { border-color: color-mix(in srgb, var(--danger) 32%, var(--hairline)); }
.ma-persona__text {
  margin: 0;
  padding: 12px;
  max-height: 340px;
  overflow: auto;
  background: var(--bg);
  border: 1px solid var(--hairline);
  border-radius: var(--radius-sm);
  font-family: var(--font-mono, monospace);
  font-size: 11.5px;
  line-height: 1.7;
  white-space: pre-wrap;
  word-break: break-word;
  color: var(--text-dim);
}
.ma-skeleton {
  background: linear-gradient(
    90deg,
    color-mix(in srgb, var(--text) 6%, transparent),
    color-mix(in srgb, var(--text) 13%, transparent),
    color-mix(in srgb, var(--text) 6%, transparent)
  );
  background-size: 200% 100%;
  animation: ma-shimmer 1.4s ease-in-out infinite;
}
.ma-skeleton--line {
  height: 12px;
  border-radius: var(--radius-sm);
  margin: 4px 0;
}
.ma-skeleton--title { height: 20px; }
@keyframes ma-shimmer {
  from { background-position: 200% 0; }
  to { background-position: -200% 0; }
}
.expert-item__id {
  font-family: var(--font-mono);
  font-size: 10.5px;
  color: var(--text-muted);
}
.expert-item.is-main { border-color: color-mix(in srgb, var(--accent) 45%, var(--hairline)); }
.ma-sr {
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  margin: -1px;
  overflow: hidden;
  clip: rect(0 0 0 0);
  white-space: nowrap;
  border: 0;
}
@media (max-width: 720px) {
  .ma-hero { flex-wrap: wrap; }
  .ma-hero__actions { flex-direction: row; flex-wrap: wrap; min-width: 0; width: 100%; }
}
@media (prefers-reduced-motion: reduce) {
  .ma-skeleton { animation: none; }
}

.expert-constellation {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}
.expert-constellation__node {
  padding: 5px 11px;
  border-radius: var(--radius-pill);
  border: 1px solid color-mix(in srgb, var(--accent) 25%, var(--hairline));
  background: color-mix(in srgb, var(--accent) 8%, var(--card));
  font-family: var(--font-mono);
  font-size: 10px;
  letter-spacing: 0.12em;
  text-transform: uppercase;
  color: var(--text-muted);
  animation: expert-node-in var(--dur-enter) var(--ease-out) both;
}
.expert-constellation__node:nth-child(1) { animation-delay: 40ms; }
.expert-constellation__node:nth-child(2) { animation-delay: 80ms; }
.expert-constellation__node:nth-child(3) { animation-delay: 120ms; }
.expert-constellation__node:nth-child(4) { animation-delay: 160ms; }
.expert-constellation__node:nth-child(5) { animation-delay: 200ms; }
.expert-constellation__node:nth-child(6) { animation-delay: 240ms; }
.expert-constellation__node:nth-child(7) { animation-delay: 280ms; }
@keyframes expert-node-in {
  from { opacity: 0; transform: translateY(6px); }
  to { opacity: 1; transform: translateY(0); }
}
.expert-error { margin: 12px 0; padding: 10px 14px; background: color-mix(in srgb, var(--danger) 10%, var(--card)); border: 1px solid color-mix(in srgb, var(--danger) 30%, transparent); border-radius: var(--radius-sm); color: var(--danger); font-size: 12px; }
.expert-card {
  margin-top: 16px;
  padding: 16px;
  background:
    linear-gradient(150deg, color-mix(in srgb, var(--accent) 7%, transparent), transparent 46%),
    var(--card);
  border: 1px solid var(--hairline);
  border-radius: var(--radius-card);
}
.expert-card__title { font-family: var(--font-display); font-size: 14px; font-weight: 700; margin-bottom: 10px; }
.expert-pills { display: flex; flex-wrap: wrap; gap: 8px; }
.expert-pill { padding: 4px 10px; border-radius: 999px; background: color-mix(in srgb, var(--text) 8%, var(--card)); font-size: 12px; }
.expert-pill--accent { color: var(--accent); }
.expert-pills--swarm { margin-top: 10px; }
.expert-field { display: flex; flex-direction: column; gap: 4px; font-size: 11px; color: var(--text-dim); }
.expert-input { width: 120px; padding: 7px 9px; border: 1px solid var(--hairline); border-radius: var(--radius-sm); background: var(--bg); color: var(--text); font-size: 12px; }
.expert-input--wide { flex: 1; min-width: 260px; width: auto; }
.expert-team { margin-top: 12px; border-top: 1px solid var(--hairline); padding-top: 10px; }
.expert-team__head { font-size: 12px; font-weight: 600; margin-bottom: 6px; }
.expert-team ul { margin: 0; padding-left: 18px; font-size: 12px; color: var(--text-dim); }
.expert-team pre { margin: 8px 0 0; padding: 10px; background: var(--bg); border: 1px solid var(--hairline); border-radius: var(--radius-sm); font-family: var(--font-mono, monospace); font-size: 11px; white-space: pre-wrap; word-break: break-all; max-height: 320px; overflow: auto; }
.expert-note--err { color: var(--danger); }
.expert-textarea { flex-basis: 100%; width: 100%; min-height: 64px; padding: 8px 10px; border: 1px solid var(--hairline); border-radius: var(--radius-sm); background: var(--bg); color: var(--text); font-size: 12px; resize: vertical; font-family: var(--font-mono, monospace); }
.expert-comp { display: flex; gap: 10px; align-items: center; padding: 7px 0; border-bottom: 1px solid var(--hairline); font-size: 12px; }
.expert-comp b { min-width: 110px; color: var(--accent); }
.expert-comp__actions { margin-left: auto; display: flex; gap: 6px; }
.expert-note { margin: 10px 0 0; font-size: 12px; color: var(--text-dim); }
.expert-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(250px, 1fr)); gap: 10px; }
.expert-item { display: flex; flex-direction: column; gap: 5px; padding: 10px; background: var(--bg); border: 1px solid var(--hairline); border-radius: var(--radius-sm); }
.expert-item b { font-size: 13px; color: var(--accent); }
.expert-item__meta { display: flex; gap: 6px; }
.expert-tag { font-style: normal; font-size: 10px; padding: 2px 7px; border-radius: 999px; background: color-mix(in srgb, var(--accent) 15%, var(--card)); color: var(--accent); }
.expert-tag--dim { background: var(--bg-hover); color: var(--text-dim); }
.expert-item__desc { font-size: 11px; color: var(--text-dim); word-break: break-all; }
.expert-item__playbook { font-size: 10px; color: var(--text-dim); }
.expert-item__cite { align-self: flex-start; margin-top: 2px; }
.expert-note--ok { color: var(--ok, var(--accent)); }
.expert-list { margin: 0; padding-left: 18px; font-size: 12px; color: var(--text-dim); line-height: 1.9; }
.expert-form { display: flex; gap: 12px; align-items: flex-end; flex-wrap: wrap; }
</style>