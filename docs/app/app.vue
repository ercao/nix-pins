<script setup lang="ts">
const route = useRoute()
const { seo } = useAppConfig()
const site = useSiteConfig()
const uiLocale = useDocsUiLocale()
const { data: navigation } = await useDocsNavigation()

provide('navigation', navigation)
useDocusShortcuts()
useHead({
  htmlAttrs: { lang: () => uiLocale.value.code, dir: () => uiLocale.value.dir },
  link: [{ rel: 'icon', href: '/favicon.ico' }],
})
useSeoMeta({
  titleTemplate: `%s - ${site.name}`,
  title: seo.title,
  description: seo.description,
  ogSiteName: site.name,
  twitterCard: 'summary_large_image',
})
</script>

<template>
  <UApp :locale="uiLocale">
    <NuxtLoadingIndicator color="var(--ui-primary)" />
    <AppHeader v-if="route.meta.header !== false" />
    <NuxtLayout>
      <NuxtPage />
    </NuxtLayout>
    <AppFooter v-if="route.meta.footer !== false" />
    <ClientOnly>
      <AppSearch :navigation="navigation" />
    </ClientOnly>
  </UApp>
</template>
