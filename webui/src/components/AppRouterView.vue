<template>
  <ErrorBoundary>
    <router-view v-slot="{ Component, route }">
      <!-- out-in: one view in the DOM at a time, so pages never overlap (no
           double-exposure, and never two composers/textareas mid-swap).
           Console views are kept-alive, so the entering page is instant —
           out-in no longer incurs the old remount/fetch "dead gap". -->
      <template v-if="route.meta.routeTransition === 'none'">
        <KeepAlive v-if="route.meta.keepAlive" :max="12">
          <component :is="Component" :key="route.meta.viewKey || route.name" />
        </KeepAlive>
        <component v-else :is="Component" :key="route.meta.viewKey || route.name" />
      </template>
      <Transition v-else name="route-fade" mode="out-in">
        <KeepAlive v-if="route.meta.keepAlive" :max="12">
          <component :is="Component" :key="route.meta.viewKey || route.name" />
        </KeepAlive>
        <component v-else :is="Component" :key="route.meta.viewKey || route.name" />
      </Transition>
    </router-view>
  </ErrorBoundary>
</template>

<script setup lang="ts">
import ErrorBoundary from './ErrorBoundary.vue'
</script>
