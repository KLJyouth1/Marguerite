<template>
  <div class="control-stage memory-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">{{ t('memoryPage.title') }}</h1>
        <p class="control-stage__subtitle">{{ t('memoryPage.subtitle') }}</p>
      </div>
    </header>

    <!-- 设置 → 记忆 里的六块面板原样上移为独立页：它们各自是自包含的
         section（自带标题/加载态/能力降级），这里只负责成列排放与统一间距。 -->
    <div class="memory-stack">
      <CrossProjectMemoryPanel />
      <KnowledgeBasePanel />
      <MemoryLayersPanel />
      <ProfileCard />
      <ExpertsPanel />
      <SettingsMemoryPanel />
    </div>
  </div>
</template>

<script setup lang="ts">
/**
 * 记忆一等页（与 /rules 并列的 Settings 分组目的地）。
 *
 * 内容就是「设置 → 记忆」那一组面板（cross-project / knowledge base /
 * 记忆分层 / 画像 / 专家 / 记忆导入），各自保留原有的 RPC 门控与降级：
 * 本页不新增任何后端契约，也不额外发 RPC —— 面板挂载时自行按能力探测。
 *
 * 滚动由外壳 `.content`（overflow-y: auto）承担，与其他 stage 页一致。
 */
import { useI18n } from 'vue-i18n'
import CrossProjectMemoryPanel from '@/components/settings/CrossProjectMemoryPanel.vue'
import KnowledgeBasePanel from '@/components/settings/KnowledgeBasePanel.vue'
import MemoryLayersPanel from '@/components/settings/MemoryLayersPanel.vue'
import ProfileCard from '@/components/settings/ProfileCard.vue'
import ExpertsPanel from '@/components/settings/ExpertsPanel.vue'
import SettingsMemoryPanel from '@/components/settings/SettingsMemoryPanel.vue'

const { t } = useI18n()
</script>

<style scoped>
/* 单一竖向节奏：面板自带 section 样式（标题/卡片），这里只定义列间距。
   880px 与面板自身的 max-width 对齐（KnowledgeBasePanel 没有自己的
   max-width，靠本容器收齐左边界）。 */
.memory-stack {
  display: flex;
  flex-direction: column;
  gap: var(--sp-6);
  margin-inline: auto;
  max-width: 880px;
  min-width: 0;
  width: 100%;
}
</style>
