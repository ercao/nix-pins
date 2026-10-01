<script setup lang="ts">
const { locale } = useI18n()
const collectionName = useDocsCollection()

const { data: files } = useLazyAsyncData(() => `docs-search-${collectionName.value}`, () =>
  queryCollectionSearchSections(collectionName.value, { ignoredTags: ['style'] }), {
  server: false,
  watch: [locale],
})
</script>

<template>
  <LazyUContentSearch :files="files" />
</template>
