<template>
  <nav
    class="mobile-tabbar"
    :class="{ 'is-keyboard-open': isKeyboardOpen }"
    :inert="inert"
    :aria-label="t('chrome.primaryMobile')"
  >
    <router-link
      to="/chat"
      class="mobile-tab"
      :class="{ 'is-active': isChatActive }"
      @click="$emit('nav')"
    >
      <Icon name="chat" :size="20" />
      <span class="mobile-tab__label">{{ t('nav.chat') }}</span>
    </router-link>
    <router-link
      to="/sessions"
      class="mobile-tab"
      :class="{ 'is-active': isSessionsActive }"
      @click="$emit('nav')"
    >
      <Icon name="sessions" :size="20" />
      <span class="mobile-tab__label">{{ t('nav.sessions') }}</span>
      <span v-if="approvalCount > 0" class="mobile-tab__badge">{{ approvalCount }}</span>
    </router-link>
    <router-link
      to="/overview"
      class="mobile-tab"
      :class="{ 'is-active': isOverviewActive }"
      @click="$emit('nav')"
    >
      <Icon name="home" :size="20" />
      <span class="mobile-tab__label">{{ t('nav.overview') }}</span>
    </router-link>
    <button
      type="button"
      class="mobile-tab"
      :class="{ 'is-active': isMoreActive }"
      @click="$emit('more')"
    >
      <Icon name="menu" :size="20" />
      <span class="mobile-tab__label">{{ t('chrome.more') }}</span>
    </button>
  </nav>
</template>

<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import Icon from './Icon.vue'

defineProps<{
  isChatActive: boolean
  isSessionsActive: boolean
  isOverviewActive: boolean
  isMoreActive: boolean
  isKeyboardOpen: boolean
  inert: boolean
  approvalCount: number
}>()

defineEmits<{
  nav: []
  more: []
}>()

const { t } = useI18n()
</script>
