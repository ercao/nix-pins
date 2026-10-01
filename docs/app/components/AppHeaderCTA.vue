<script setup lang="ts">
const { locales } = useI18n()
</script>

<template>
  <template v-if="locales.length === 1">
    <LanguageSelect />
    <USeparator orientation="vertical" class="h-8" />
  </template>
</template>
