import { MetadataRoute } from "next";
import { client } from "@/lib/sanity";

const BASE_URL = process.env.NEXT_PUBLIC_APP_URL || "https://planoai.dev";

interface BlogPost {
  slug: { current: string };
  publishedAt?: string;
  _updatedAt?: string;
}

async function getBlogPosts(): Promise<BlogPost[]> {
  const query = `*[_type == "blog" && published == true] | order(publishedAt desc) {
    slug,
    publishedAt,
    _updatedAt
  }`;

  try {
    return await client.fetch(query);
  } catch (error) {
    console.error("Error fetching blog posts for sitemap:", error);
    return [];
  }
}

export default async function sitemap(): Promise<MetadataRoute.Sitemap> {
  // Static pages with their priorities and change frequencies
  const staticPages: MetadataRoute.Sitemap = [
    {
      url: BASE_URL,
      lastModified: new Date(),
      changeFrequency: "weekly",
      priority: 1.0,
    },
    {
      url: `${BASE_URL}/research`,
      lastModified: new Date(),
      changeFrequency: "monthly",
      priority: 0.9,
    },
    {
      url: `${BASE_URL}/blog`,
      lastModified: new Date(),
      changeFrequency: "daily",
      priority: 0.8,
    },
    {
      url: `${BASE_URL}/contact`,
      lastModified: new Date(),
      changeFrequency: "monthly",
      priority: 0.6,
    },
    {
      url: `${BASE_URL}/docs`,
      lastModified: new Date(),
      changeFrequency: "weekly",
      priority: 0.7,
    },
  ];

  // Fetch dynamic blog posts
  const blogPosts = await getBlogPosts();

  const blogPages: MetadataRoute.Sitemap = blogPosts.map((post) => ({
    url: `${BASE_URL}/blog/${post.slug.current}`,
    lastModified: post._updatedAt
      ? new Date(post._updatedAt)
      : post.publishedAt
        ? new Date(post.publishedAt)
        : new Date(),
    changeFrequency: "monthly" as const,
    priority: 0.7,
  }));

  return [...staticPages, ...blogPages];
}
