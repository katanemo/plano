import { Metadata } from "next";
import { client } from "@/lib/sanity";

type Params = Promise<{ slug: string }>;

interface BlogPost {
  _id: string;
  title: string;
  slug: { current: string };
  summary?: string;
  publishedAt?: string;
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
    publishedAt,
    author
  }`;

  const post = await client.fetch(query, { slug });
  return post || null;
}

export async function generateMetadata({
  params,
}: {
  params: Params;
}): Promise<Metadata> {
  try {
    const resolvedParams = await params;
    const post = await getBlogPost(resolvedParams.slug);

    if (!post) {
      return {
        title: "Post Not Found - Plano",
        description: "The requested blog post could not be found.",
      };
    }

    // Get baseUrl - use NEXT_PUBLIC_APP_URL if set, otherwise construct from VERCEL_URL
    // Restrict to allowed hosts: localhost:3000, archgw-tau.vercel.app, or planoai.dev
    let baseUrl = "http://localhost:3000";

    if (process.env.NEXT_PUBLIC_APP_URL) {
      try {
        const parsed = new URL(process.env.NEXT_PUBLIC_APP_URL);
        const allowedHosts = new Set([
          "archgw-tau.vercel.app",
          "planoai.dev",
          "localhost",
        ]);
        if (allowedHosts.has(parsed.hostname)) {
          baseUrl = parsed.origin;
        }
      } catch {
        // Invalid URL, keep default
      }
    } else if (process.env.VERCEL_URL) {
      const hostname = process.env.VERCEL_URL;
      if (
        hostname === "archgw-tau.vercel.app" ||
        hostname === "planoai.dev"
      ) {
        baseUrl = `https://${hostname}`;
      }
    }

    const ogImageUrl = `${baseUrl}/api/og/${resolvedParams.slug}`;

    const metadata: Metadata = {
      title: `${post.title} - Plano Blog`,
      description: post.summary || "Read more on Plano Blog",
      openGraph: {
        title: post.title,
        description: post.summary || "Read more on Plano Blog",
        type: "article",
        publishedTime: post.publishedAt,
        authors: post.author?.name ? [post.author.name] : undefined,
        url: `${baseUrl}/blog/${resolvedParams.slug}`,
        siteName: "Plano",
        images: [
          {
            url: ogImageUrl,
            width: 1200,
            height: 630,
            alt: post.title,
          },
        ],
        locale: "en_US",
      },
      twitter: {
        card: "summary_large_image",
        title: post.title,
        description: post.summary || "Read more on Plano Blog",
        images: [ogImageUrl],
      },
    };

    return metadata;
  } catch (error) {
    console.error("Error generating metadata:", error);
    return {
      title: "Blog Post - Plano",
      description: "Read this post on Plano Blog",
    };
  }
}

interface LayoutProps {
  children: React.ReactNode;
  params: Params;
}

export default async function Layout({ children, params }: LayoutProps) {
  return <>{children}</>;
}
