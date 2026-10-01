<script setup lang="ts">
const { locale, locales } = useI18n()
const localePath = useLocalePath()
const switchLocalePath = useSwitchLocalePath()
const currentLocale = computed(() => locales.value.find(item => item.code === locale.value))
const items = computed(() => locales.value.map(item => ({
  label: item.name || item.code,
  to: switchLocalePath(item.code) || localePath('/'),
})))
</script>

<template>
  <UDropdownMenu :items="items" :content="{ align: 'end' }">
    <UButton
      icon="i-lucide-languages"
      color="neutral"
      variant="ghost"
      :aria-label="currentLocale?.name || locale"
      class="size-8"
    />
  </UDropdownMenu>
</template>
