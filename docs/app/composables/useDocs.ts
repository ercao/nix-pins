import type { PageCollections } from '@nuxt/content'
import * as uiLocales from '@nuxt/ui/locale'

export function useDocsCollection() {
  const { locale } = useNuxtApp().$i18n
  return computed(() => `docs_${locale.value.replace('-', '_')}` as Extract<keyof PageCollections, `docs_${string}`>)
}

export function useDocsUiLocale() {
  const { locale } = useI18n()
  return computed(() => uiLocales[locale.value.replace('-', '_').toLowerCase() as keyof typeof uiLocales] || uiLocales.en)
}

export function useDocsNavigation() {
  const { locale } = useNuxtApp().$i18n
  const collection = useDocsCollection()
  return useAsyncData(() => `navigation_${collection.value}`, () => queryCollectionNavigation(collection.value), {
    transform: navigation => navigation.find(item => item.path === `/${locale.value}`)?.children ?? [],
    watch: [locale],
  })
}
