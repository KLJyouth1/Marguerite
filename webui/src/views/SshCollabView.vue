<template>
  <div class="control-stage ssh-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">企业协同 · SSH</h1>
        <p class="control-stage__subtitle">主机台账 + 连接探测（复用部署/终端 SSH 工具链；不存密码，仅存密钥路径）</p>
      </div>
      <div class="control-stage__actions"><button class="btn btn--ghost" @click="load"><Icon name="refresh" :size="16" /><span>刷新</span></button></div>
    </header>
    <section v-if="error" class="ssh-error">{{ error }}</section>
    <section class="ssh-card">
      <div class="ssh-card__title">主机列表（collab.ssh.list）</div>
      <table v-if="hosts.length" class="ssh-table">
        <thead><tr><th>名称</th><th>地址</th><th>用户/端口</th><th>密钥</th><th>动作</th></tr></thead>
        <tbody>
          <tr v-for="h in hosts" :key="h.name">
            <td><b>{{ h.name }}</b></td>
            <td>{{ h.host }}</td>
            <td>{{ h.user }}@{{ h.host }}:{{ h.port }}</td>
            <td>{{ h.key || '（默认认证）' }}</td>
            <td class="ssh-actions">
              <button class="btn btn--sm btn--ghost" :disabled="busy" @click="probe(h.name)">探测</button>
              <button class="btn btn--sm btn--ghost" :disabled="busy" @click="remove(h.name)">删除</button>
            </td>
          </tr>
        </tbody>
      </table>
      <div v-else class="ssh-empty">未配置 SSH 主机</div>
      <div v-for="(text, n) in probeResults" :key="n" class="ssh-probe"><b>{{ n }}</b>：{{ text }}</div>
      <div class="ssh-form">
        <div class="ssh-card__title">新增主机</div>
        <div class="ssh-form__row">
          <input v-model="form.name" class="ssh-input" placeholder="名称" />
          <input v-model="form.host" class="ssh-input" placeholder="host（如 10.0.0.2）" />
          <input v-model="form.user" class="ssh-input" placeholder="user" />
          <input v-model="form.port" class="ssh-input" placeholder="port（22）" />
          <input v-model="form.key" class="ssh-input" placeholder="密钥路径（可选）" />
          <button class="btn btn--sm btn--primary" :disabled="busy || !form.name || !form.host || !form.user" @click="add">新增</button>
        </div>
      </div>
      <div class="ssh-form">
        <div class="ssh-card__title">远程执行（collab.ssh.exec）</div>
        <div class="ssh-form__row">
          <select v-model="execHost" class="ssh-input">
            <option value="" disabled>选择主机</option>
            <option v-for="h in hosts" :key="h.name" :value="h.name">{{ h.name }}（{{ h.user }}@{{ h.host }}）</option>
          </select>
          <input v-model="execCmd" class="ssh-input ssh-exec-cmd" placeholder="命令（如 uptime / df -h / systemctl status）" @keyup.enter="runExec" />
          <button class="btn btn--sm btn--primary" :disabled="busy || !execHost || !execCmd.trim()" @click="runExec">执行</button>
        </div>
        <div v-if="execResult" class="ssh-exec-out">
          <div class="ssh-exec-out__head">退出码 {{ execResult.exitCode ?? "—" }} <span :class="execResult.ok ? 'ssh-exec-ok' : 'ssh-exec-fail'">{{ execResult.ok ? '成功' : '失败' }}</span></div>
          <pre v-if="execResult.stdout">{{ execResult.stdout }}</pre>
          <pre v-if="execResult.stderr" class="ssh-exec-err">{{ execResult.stderr }}</pre>
        </div>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'
import { useConfirm } from '@/composables/useConfirm'
const rpc = useRpcStore()
const { confirm } = useConfirm()
interface Host { name: string; host: string; port: number; user: string; key?: string | null }
interface Hosts { ok: boolean; entries: Host[]; error?: string }
interface Probe { ok: boolean; message?: string; error?: string }
const hosts = ref<Host[]>([])
const form = ref({ name: '', host: '', user: '', port: '22', key: '' })
const probeResults = ref<Record<string, string>>({})
const busy = ref(false)
const error = ref('')
async function safe<T>(fn: () => Promise<T>): Promise<T | null> { try { return await fn() } catch (e) { error.value = e instanceof Error ? e.message : String(e); return null } }
async function load() {
  const r = await safe(() => rpc.call<Hosts>('collab.ssh.list', {}))
  if (r) hosts.value = r.entries || []
}
async function add() {
  busy.value = true
  const r = await safe(() => rpc.call<Hosts>('collab.ssh.add', { name: form.value.name, host: form.value.host, user: form.value.user, port: Number(form.value.port || 22), key: form.value.key }))
  if (r) { hosts.value = r.entries || []; form.value = { name: '', host: '', user: '', port: '22', key: '' } }
  busy.value = false
}
async function remove(name: string) {
  const target = hosts.value.find(h => h.name === name)
  const where = target ? `（${target.user}@${target.host}:${target.port}）` : ''
  const approved = await confirm({
    title: '删除 SSH 主机',
    body: `将从主机台账删除「${name}」${where}。仅移除本地台账记录，不会改动远端主机本身。`,
    primaryLabel: '删除',
  })
  if (!approved) return
  busy.value = true
  const r = await safe(() => rpc.call<Hosts>('collab.ssh.remove', { name }))
  if (r) hosts.value = r.entries || []
  busy.value = false
}
async function probe(name: string) {
  busy.value = true
  const r = await safe(() => rpc.call<Probe>('collab.ssh.probe', { name }))
  if (r) probeResults.value[name] = r.ok ? (r.message || 'OK') : (r.error || '失败')
  busy.value = false
}
interface ExecResult { ok: boolean; exitCode?: number | null; stdout?: string; stderr?: string; error?: string }
const execHost = ref('')
const execCmd = ref('')
const execResult = ref<ExecResult | null>(null)
/** 确认框里的命令压成单行并截断，避免超长命令撑爆弹窗。 */
function clipCommand(text: string, max = 160): string {
  const flat = text.replace(/\s+/g, ' ').trim()
  return flat.length > max ? `${flat.slice(0, max)}…` : flat
}
async function runExec() {
  if (!execHost.value || !execCmd.value.trim()) return
  const command = execCmd.value.trim()
  const target = hosts.value.find(h => h.name === execHost.value)
  const where = target ? `${target.name}（${target.user}@${target.host}:${target.port}）` : execHost.value
  const approved = await confirm({
    title: '确认远程执行',
    body: `将在 ${where} 上执行命令：\n${clipCommand(command)}\n命令会直接在远端主机运行。`,
    primaryLabel: '执行',
  })
  if (!approved) return
  busy.value = true
  execResult.value = null
  const r = await safe(() => rpc.call<ExecResult>('collab.ssh.exec', { name: execHost.value, command }))
  if (r) execResult.value = r
  busy.value = false
}

onMounted(load)
</script>

<style scoped>
.ssh-stage { padding: 24px 28px; max-width: 1100px; margin: 0 auto; }
.ssh-error { margin: 12px 0; padding: 10px 14px; background: color-mix(in srgb, var(--danger) 10%, var(--card)); border: 1px solid color-mix(in srgb, var(--danger) 30%, transparent); border-radius: var(--radius-sm); color: var(--danger); font-size: 12px; }
.ssh-card { margin-top: 14px; padding: 14px; background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); }
.ssh-card__title { font-size: 13px; font-weight: 600; margin-bottom: 10px; }
.ssh-table { width: 100%; border-collapse: collapse; font-size: 12px; }
.ssh-table th, .ssh-table td { text-align: left; padding: 7px 8px; border-bottom: 1px solid var(--hairline); }
.ssh-actions { display: flex; gap: 8px; }
.ssh-empty { padding: 16px; text-align: center; color: var(--text-dim); }
.ssh-probe { margin-top: 8px; font-size: 12px; color: var(--text-dim); }
.ssh-form { margin-top: 14px; border-top: 1px solid var(--hairline); padding-top: 10px; }
.ssh-form__row { display: flex; gap: 8px; flex-wrap: wrap; }
.ssh-input { flex: 1; min-width: 120px; padding: 7px 9px; border: 1px solid var(--hairline); border-radius: var(--radius-sm); background: var(--bg); color: var(--text); font-size: 12px; }
.ssh-exec-cmd { flex: 3; }
.ssh-exec-out { margin-top: 10px; padding: 10px; background: var(--bg); border: 1px solid var(--hairline); border-radius: var(--radius-sm); }
.ssh-exec-out__head { font-size: 12px; margin-bottom: 6px; }
.ssh-exec-out pre { margin: 6px 0 0; font-family: var(--font-mono); font-size: 11px; white-space: pre-wrap; word-break: break-all; }
.ssh-exec-ok { color: var(--ok); }
.ssh-exec-fail { color: var(--danger); }
.ssh-exec-err { color: var(--danger); }
</style>
