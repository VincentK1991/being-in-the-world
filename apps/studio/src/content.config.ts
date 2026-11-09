import { defineCollection } from 'astro:content';
import { z } from 'astro/zod';
import { docsLoader, i18nLoader } from '@astrojs/starlight/loaders';
import { docsSchema, i18nSchema } from '@astrojs/starlight/schema';

export const collections = {
  i18n: defineCollection({ loader: i18nLoader(), schema: i18nSchema() }),
  docs: defineCollection({
    loader: docsLoader(),
    schema: docsSchema({
      extend: z.object({
        // Internals pages record what they were last checked against (docs/specs/studio-docs.md, D8).
        verified: z.object({
          commit: z.string(),
          refs: z.array(z.string()),
          oxigraph: z.string().optional(),
        }).optional(),
      }),
    }),
  }),
};
