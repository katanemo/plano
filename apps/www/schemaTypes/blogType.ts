import { defineField, defineType } from "sanity";

export const blogType = defineType({
  name: "blog",
  title: "Blog",
  type: "document",
  fields: [
    defineField({
      name: "title",
      title: "Title",
      type: "string",
      validation: (rule) => rule.required(),
    }),
    defineField({
      name: "slug",
      title: "Slug",
      type: "slug",
      options: {
        source: "title",
        maxLength: 96,
      },
      validation: (rule) => rule.required(),
    }),
    defineField({
      name: "summary",
      title: "Post Summary",
      type: "text",
      description: "A brief summary of the blog post",
    }),
    defineField({
      name: "body",
      title: "Post Body",
      type: "array",
      of: [
        {
          type: "block",
        },
        {
          type: "code",
          options: {
            language: "typescript",
            languageAlternatives: [
              { title: "TypeScript", value: "typescript" },
              { title: "JavaScript", value: "javascript" },
              { title: "HTML", value: "html" },
              { title: "CSS", value: "css" },
              { title: "Bash", value: "sh" },
              { title: "Python", value: "python" },
              { title: "Markdown", value: "markdown" },
              { title: "YAML", value: "yaml" },
              { title: "JSON", value: "json" },
              { title: "XML", value: "xml" },
              { title: "SQL", value: "sql" },
              { title: "Shell", value: "shell" },
              { title: "PowerShell", value: "powershell" },
              { title: "Batch", value: "batch" },
            ],
            withFilename: true,
          },
        },
        {
          type: "object",
          name: "markdownBlock",
          title: "Markdown",
          fields: [
            {
              name: "markdown",
              title: "Markdown",
              type: "markdown",
              description: "Markdown content with preview and image uploads",
            },
          ],
        },
        {
          type: "table",
        },
        {
          type: "image",
          fields: [
            {
              name: "alt",
              type: "string",
              title: "Alternative text",
            },
          ],
        },
      ],
    }),
    defineField({
      name: "bodyHtml",
      title: "Post Body (HTML)",
      type: "text",
      description: "Raw HTML content from migration (for reference)",
      hidden: true,
    }),
    defineField({
      name: "mainImage",
      title: "Main Image",
      type: "image",
      options: {
        hotspot: true,
      },
      fields: [
        {
          name: "alt",
          type: "string",
          title: "Alternative text",
        },
      ],
    }),
    defineField({
      name: "mainImageUrl",
      title: "Main Image URL",
      type: "url",
      description: "Fallback URL if image not uploaded to Sanity",
      hidden: true,
    }),
    defineField({
      name: "thumbnailImage",
      title: "Thumbnail Image",
      type: "image",
      options: {
        hotspot: true,
      },
      fields: [
        {
          name: "alt",
          type: "string",
          title: "Alternative text",
        },
      ],
    }),
    defineField({
      name: "thumbnailImageUrl",
      title: "Thumbnail Image URL",
      type: "url",
      description: "Fallback URL if image not uploaded to Sanity",
      hidden: true,
    }),
    defineField({
      name: "featured",
      title: "Featured?",
      type: "boolean",
      initialValue: false,
    }),
    defineField({
      name: "published",
      title: "Published",
      type: "boolean",
      initialValue: false,
    }),
    defineField({
      name: "draft",
      title: "Draft",
      type: "boolean",
      initialValue: false,
    }),
    defineField({
      name: "archived",
      title: "Archived",
      type: "boolean",
      initialValue: false,
    }),
    defineField({
      name: "publishedAt",
      title: "Published On",
      type: "datetime",
      validation: (rule) => rule.required(),
    }),
    defineField({
      name: "createdAt",
      title: "Created On",
      type: "datetime",
      initialValue: () => new Date().toISOString(),
    }),
    defineField({
      name: "updatedAt",
      title: "Updated On",
      type: "datetime",
      initialValue: () => new Date().toISOString(),
    }),
    defineField({
      name: "author",
      title: "Author",
      type: "object",
      fields: [
        {
          name: "name",
          title: "Author Name",
          type: "string",
          validation: (rule) => rule.required(),
        },
        {
          name: "title",
          title: "Author Title",
          type: "string",
        },
        {
          name: "image",
          title: "Author Image",
          type: "image",
          options: {
            hotspot: true,
          },
        },
        {
          name: "imageUrl",
          title: "Author Image URL",
          type: "url",
          description: "Fallback URL if image not uploaded to Sanity",
          hidden: true,
        },
      ],
    }),
    defineField({
      name: "color",
      title: "Color",
      type: "string",
      description: "Optional color theme for the blog post",
    }),
    // Legacy fields from CSV migration (can be hidden in UI)
    defineField({
      name: "collectionId",
      title: "Collection ID",
      type: "string",
      hidden: true,
    }),
    defineField({
      name: "localeId",
      title: "Locale ID",
      type: "string",
      hidden: true,
    }),
    defineField({
      name: "itemId",
      title: "Item ID",
      type: "string",
      hidden: true,
    }),
  ],
  preview: {
    select: {
      title: "title",
      author: "author.name",
      media: "mainImage",
      publishedAt: "publishedAt",
    },
    prepare(selection) {
      const { author } = selection;
      return { ...selection, subtitle: author && `by ${author}` };
    },
  },
});
