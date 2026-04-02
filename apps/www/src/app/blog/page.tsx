import { client } from "@/lib/sanity";
import type { Metadata } from "next";
import { UnlockPotentialSection } from "@/components/UnlockPotentialSection";
import { BlogHeader } from "@/components/BlogHeader";
import { FeaturedBlogCard } from "@/components/FeaturedBlogCard";
import { BlogCard } from "@/components/BlogCard";
import { BlogSectionHeader } from "@/components/BlogSectionHeader";
import { pageMetadata } from "@/lib/metadata";

export const metadata: Metadata = pageMetadata.blog;
export const dynamic = "force-dynamic";

interface BlogPost {
  _id: string;
  title: string;
  slug: { current: string };
  summary?: string;
  publishedAt?: string;
  mainImage?: any;
  mainImageUrl?: string;
  thumbnailImage?: any;
  thumbnailImageUrl?: string;
  author?: {
    name?: string;
    title?: string;
    image?: any;
  };
  featured?: boolean;
}

function formatDate(dateString: string): string {
  const date = new Date(dateString);
  const day = date.getDate();
  const month = date.toLocaleDateString("en-US", { month: "long" });
  const year = date.getFullYear();

  // Add ordinal suffix
  const getOrdinal = (n: number) => {
    const s = ["th", "st", "nd", "rd"];
    const v = n % 100;
    return n + (s[(v - 20) % 10] || s[v] || s[0]);
  };

  return `${month} ${getOrdinal(day)}, ${year}`;
}

async function getBlogPosts(): Promise<BlogPost[]> {
  if (!client) {
    return [];
  }

  const query = `*[_type == "blog" && published == true] | order(publishedAt desc) {
    _id,
    title,
    slug,
    summary,
    publishedAt,
    mainImage,
    mainImageUrl,
    thumbnailImage,
    thumbnailImageUrl,
    author,
    featured
  }`;

  try {
    return await client.fetch(query);
  } catch (error) {
    console.error("Error fetching blog posts:", error);
    return [];
  }
}

async function getFeaturedBlogPost(): Promise<BlogPost | null> {
  if (!client) {
    return null;
  }

  const query = `*[_type == "blog" && published == true && featured == true] | order(_updatedAt desc, publishedAt desc)[0] {
    _id,
    title,
    slug,
    summary,
    publishedAt,
    mainImage,
    mainImageUrl,
    thumbnailImage,
    thumbnailImageUrl,
    author,
    featured
  }`;

  try {
    const post = await client.fetch(query);
    return post || null;
  } catch (error) {
    console.error("Error fetching featured blog post:", error);
    return null;
  }
}

export default async function BlogPage() {
  const [posts, featuredCandidate] = await Promise.all([
    getBlogPosts(),
    getFeaturedBlogPost(),
  ]);
  const featuredPost = featuredCandidate || posts[0];
  const recentPosts = posts
    .filter((post) => post._id !== featuredPost?._id)
    .slice(0, 3);

  // Format dates in server component
  const featuredPostWithDate = featuredPost
    ? {
        ...featuredPost,
        formattedDate: featuredPost.publishedAt
          ? formatDate(featuredPost.publishedAt)
          : undefined,
      }
    : null;

  const recentPostsWithDates = recentPosts.map((post) => ({
    ...post,
    formattedDate: post.publishedAt ? formatDate(post.publishedAt) : undefined,
  }));

  return (
    <div className="min-h-screen">
      {/* Header Section */}
      <BlogHeader />

      {/* Featured Post */}
      {featuredPostWithDate && (
        <section className="">
          <div className="max-w-[85rem] mx-auto px-4 sm:px-6 lg:px-8 pb-8 sm:pb-12 lg:pb-0">
            <FeaturedBlogCard post={featuredPostWithDate} />
          </div>
        </section>
      )}

      {/* Recent Posts Section */}
      {recentPostsWithDates.length > 0 && (
        <section className="border-b border-black/10 py-8 sm:py-12 lg:py-24">
          <div className="max-w-[85rem] mx-auto px-4 sm:px-6 lg:px-8">
            <BlogSectionHeader />
            <div className="grid sm:grid-cols-2 lg:grid-cols-3 gap-6 lg:gap-8">
              {recentPostsWithDates.map((post, index) => (
                <BlogCard key={post._id} post={post} index={index} />
              ))}
            </div>
          </div>
        </section>
      )}

      {/* Call to Action Section */}
      <UnlockPotentialSection variant="transparent" />
    </div>
  );
}
