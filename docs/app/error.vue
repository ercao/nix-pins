<script setup lang="ts">
import type { NuxtError } from '#app'

const props = defineProps<{ error: NuxtError }>()
const { t } = useI18n()
const uiLocale = useDocsUiLocale()
const { data: navigation } = await useDocsNavigation()
const error = computed(() => ({
  ...props.error,
  statusMessage: t('common.error.title'),
  message: t('common.error.description'),
}))

provide('navigation', navigation)
useHead({ htmlAttrs: { lang: () => uiLocale.value.code, dir: () => uiLocale.value.dir } })
useSeoMeta({ title: () => t('common.error.title'), description: () => t('common.error.description') })
</script>

<template>
  <UApp :locale="uiLocale">
    <AppHeader />
    <UError :error="error" />
    <AppFooter />
    <ClientOnly>
      <AppSearch :navigation="navigation" />
    </ClientOnly>
  </UApp>
</template>
