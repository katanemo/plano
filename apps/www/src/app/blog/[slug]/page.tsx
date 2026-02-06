import { client, urlFor } from "@/lib/sanity";
import type { Metadata } from "next";
import Image from "next/image";
import Link from "next/link";
import { PortableText } from "@/components/PortableText";
import { notFound } from "next/navigation";
import { UnlockPotentialSection } from "@/components/UnlockPotentialSection";
import { createBlogPostMetadata } from "@/lib/metadata";

interface BlogPost {
  _id: string;
  title: string;
  slug: { current: string };
  summary?: string;
  body?: any[];
  bodyHtml?: string;
  publishedAt?: string;
  mainImage?: any;
  mainImageUrl?: string;
  author?: {
    name?: string;
    title?: string;
    image?: any;
  };
}

async function getBlogPost(slug: string): Promise<BlogPost | null> {
  const query = `*[_type == "blog" && slug.current == $slug && published == true][0] {
    _id,
    title,
    slug,
    summary,
    body[]{
      ...,
      asset->{
        _id,
        url,
        metadata {
          dimensions {
            width,
            height,
            aspectRatio
          }
        }
      }
    },
    bodyHtml,
    publishedAt,
    mainImage,
    mainImageUrl,
    author
  }`;

  const post = await client.fetch(query, { slug });
  return post || null;
}

async function getAllBlogSlugs(): Promise<string[]> {
  const query = `*[_type == "blog" && published == true] {
    "slug": slug.current
  }`;

  const posts = await client.fetch(query);
  return posts.map((post: { slug: string }) => post.slug);
}

export async function generateStaticParams() {
  const slugs = await getAllBlogSlugs();
  return slugs.map((slug) => ({ slug }));
}

export async function generateMetadata({
  params,
}: {
  params: Promise<{ slug: string }>;
}): Promise<Metadata> {
  const { slug } = await params;
  const post = await getBlogPost(slug);

  if (!post) {
    return {
      title: "Post Not Found | Plano Blog",
      description: "The requested blog post could not be found.",
    };
  }

  return createBlogPostMetadata({
    title: post.title,
    description: post.summary,
    slug: post.slug.current,
    publishedAt: post.publishedAt,
    author: post.author?.name,
  });
}

export default async function BlogPostPage({
  params,
}: {
  params: Promise<{ slug: string }>;
}) {
  const { slug } = await params;
  const post = await getBlogPost(slug);

  if (!post) {
    notFound();
  }

  return (
    <article className="min-h-screen">
      {/* Featured Image - First */}
      {(post.mainImage || post.mainImageUrl) && (
        <div className="">
          <div className="max-w-[89rem] mx-auto px-4 sm:px-6 lg:px-8 pt-8 sm:pt-12 lg:pt-1 pb-8 sm:pb-12">
            <div className="relative aspect-[21/8] w-full overflow-hidden rounded-lg">
              {post.mainImage ? (
                <Image
                  src={urlFor(post.mainImage).width(1600).url()}
                  alt={post.title}
                  fill
                  className="object-cover"
                  priority
                />
              ) : (
                <Image
                  src={post.mainImageUrl!}
                  alt={post.title}
                  fill
                  className="object-cover"
                  priority
                />
              )}
            </div>
          </div>
        </div>
      )}

      {/* Content Section */}
      <div className="max-w-[58rem] mx-auto px-4 sm:px-6 lg:px-8">
        {/* Back to Blog Button */}
        <div className="pt-4 sm:pt-6 lg:pt-8 pb-4 sm:pb-6">
          <Link
            href="/blog"
            className="inline-flex items-center gap-2 text-sm font-medium text-black/60 hover:text-black transition-colors"
          >
            <svg
              className="w-4 h-4"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M15 19l-7-7 7-7"
              />
            </svg>
            Back to Blog
          </Link>
        </div>

        {/* Author and Date */}
        <div className="flex flex-col sm:flex-row items-start sm:items-center justify-between gap-4 sm:gap-6 pb-4">
          {post.author?.name && (
            <div className="flex items-center gap-3">
              {post.author.image ? (
                <div className="relative w-12 h-12 rounded overflow-hidden shrink-0">
                  <Image
                    src={urlFor(post.author.image).width(80).url()}
                    alt={post.author.name}
                    fill
                    className="object-cover"
                  />
                </div>
              ) : (
                <div className="w-12 h-12 rounded bg-[var(--secondary)]/20 shrink-0" />
              )}
              <div>
                <div className="text-lg font-mono font-semibold tracking-wider text-primary uppercase">
                  {post.author.name}
                </div>
                {post.author.title && (
                  <div className="text-sm font-mono font-normal tracking-wider text-[#28327D] uppercase">
                    {post.author.title}
                  </div>
                )}
              </div>
            </div>
          )}
          {post.publishedAt && (
            <time
              dateTime={post.publishedAt}
              className="text-base font-medium tracking-[-0.9px] text-black sm:ml-auto"
            >
              {new Date(post.publishedAt).toLocaleDateString("en-US", {
                year: "numeric",
                month: "long",
                day: "numeric",
              })}
            </time>
          )}
        </div>

        {/* Title */}
        <div className="pb-6 sm:pb-8 sm:-ml-1.5">
          <h1 className="text-4xl sm:text-5xl lg:text-6xl font-medium leading-tight tracking-tighter text-black">
            <span className="font-sans">{post.title}</span>
          </h1>
        </div>

        {/* Content */}
        <div className="pb-12 sm:pb-16 lg:pb-20 ">
          {post.body && post.body.length > 0 ? (
            <div className="prose prose-lg max-w-none">
              <PortableText content={post.body} />
            </div>
          ) : post.bodyHtml ? (
            <div
              className="prose prose-lg max-w-none"
              dangerouslySetInnerHTML={{ __html: post.bodyHtml }}
            />
          ) : (
            <p className="text-base sm:text-lg font-sans font-normal tracking-[-0.5px] text-black/80">
              Content coming soon...
            </p>
          )}
        </div>
      </div>
      <UnlockPotentialSection variant="transparent" />
    </article>
  );
}
