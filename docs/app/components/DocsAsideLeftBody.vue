<script setup lang="ts">
const { sidebarNavigation } = useSubNavigation()
const variants = useUIConfig('contentNavigation')
const navigationProps = computed(() => ({
  collapsible: false,
  highlight: variants.value.highlight ?? true,
  highlightColor: variants.value.highlightColor,
  variant: variants.value.variant ?? 'link',
  color: variants.value.color,
}))
</script>

<template>
  <nav class="space-y-1.5">
    <div v-for="section in sidebarNavigation" :key="section.path">
      <UContentNavigation
        v-bind="navigationProps"
        as="div"
        :navigation="[{ ...section, children: undefined }]"
        :ui="{ link: 'font-semibold text-highlighted' }"
      />
      <UContentNavigation
        v-if="section.children?.length"
        v-bind="navigationProps"
        :navigation="section.children"
        :level="1"
      />
    </div>
  </nav>
</template>
