<script setup lang="ts">
import type { ContentNavigationItem } from '@nuxt/content'
import { findPageHeadline } from '@nuxt/content/utils'

definePageMeta({
  layout: 'docs',
  path: '/:slug(.*)+',
  middleware: async (to) => {
    const { data: navigation } = await useDocsNavigation()
    const section = navigation.value?.find(item => item.path === to.path.replace(/\/$/, ''))
    const firstPage = section?.children?.[0]?.path
    if (firstPage) return navigateTo(firstPage, { replace: true, redirectCode: 302 })
  },
})

const route = useRoute()
const { t } = useI18n()
const { github } = useAppConfig()
const navigation = inject<Ref<ContentNavigationItem[] | null>>('navigation')
const collection = useDocsCollection()
const [{ data: page }, { data: surround }] = await Promise.all([
  useAsyncData(() => route.path, () => queryCollection(collection.value).path(route.path).first()),
  useAsyncData(() => `${route.path}-surround`, () => queryCollectionItemSurroundings(collection.value, route.path, {
    fields: ['description'],
  })),
])

if (!page.value) {
  throw createError({ statusCode: 404, statusMessage: 'Page not found', fatal: true })
}

const headline = computed(() => findPageHeadline(navigation?.value ?? [], page.value?.path))
const breadcrumbs = computed(() => findPageBreadcrumbs(navigation?.value ?? [], page.value?.path ?? ''))
const editLink = computed(() => github ? [
  github.url, 'edit', github.branch, github.rootDir, 'content',
  `${page.value?.stem}.${page.value?.extension}`,
].filter(Boolean).join('/') : undefined)

useSeo({
  title: page.value.seo?.title || page.value.title,
  description: page.value.seo?.description || page.value.description,
  type: 'article',
  breadcrumbs,
})
addPrerenderPath(`/raw${route.path}.md`)
</script>

<template>
  <UPage v-if="page">
    <UPageHeader
      :title="page.title"
      :description="page.description"
      :headline="headline"
      :ui="{ wrapper: 'flex-row items-center flex-wrap justify-between' }"
    >
      <template #links>
        <UButton v-for="(link, index) in page.links" :key="index" size="sm" v-bind="link" />
        <DocsPageHeaderLinks />
      </template>
    </UPageHeader>
    <UPageBody>
      <ContentRenderer :value="page" />
      <USeparator v-if="github">
        <div class="flex items-center gap-2 text-sm text-muted">
          <UButton variant="link" color="neutral" :to="editLink" target="_blank" icon="i-lucide-pen" :ui="{ leadingIcon: 'size-4' }">
            {{ t('docs.edit') }}
          </UButton>
          <template v-if="github.url">
            <span>{{ t('common.or') }}</span>
            <UButton variant="link" color="neutral" :to="`${github.url}/issues/new/choose`" target="_blank" icon="i-lucide-alert-circle" :ui="{ leadingIcon: 'size-4' }">
              {{ t('docs.report') }}
            </UButton>
          </template>
        </div>
      </USeparator>
      <UContentSurround :surround="surround" />
    </UPageBody>
    <template #right>
      <DocsAsideRight :page="page" />
    </template>
  </UPage>
</template>
