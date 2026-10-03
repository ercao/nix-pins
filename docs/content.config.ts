import { defineCollection, defineContentConfig, z } from '@nuxt/content'

// 集合名须为有效的 JS 标识符，目录与 URL 保留 zh-CN。
export default defineContentConfig({
  collections: {
    docs_en: defineCollection({
      type: 'page',
      source: {
        include: 'en/**/*',
        prefix: '/en',
      },
      schema: z.object({
        links: z.array(z.object({
          label: z.string(),
          icon: z.string(),
          to: z.string(),
          target: z.string().optional(),
        })).optional(),
      }),
    }),
    docs_zh_CN: defineCollection({
      type: 'page',
      source: {
        include: 'zh-CN/**/*',
        prefix: '/zh-CN',
      },
      schema: z.object({
        links: z.array(z.object({
          label: z.string(),
          icon: z.string(),
          to: z.string(),
          target: z.string().optional(),
        })).optional(),
      }),
    }),
  },
})
