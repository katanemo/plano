import type { Metadata } from "next";

const BASE_URL = process.env.NEXT_PUBLIC_APP_URL || "https://planoai.dev";

/**
 * Site-wide metadata configuration
 * Centralized SEO settings for consistent branding and search optimization
 */
export const siteConfig = {
  name: "Plano",
  tagline: "Delivery Infrastructure for Agentic Apps",
  description:
    "Build agents faster and deliver them reliably to production. Plano is an AI-native proxy and data plane for agent orchestration, LLM routing, guardrails, and observability.",
  url: BASE_URL,
  ogImage: `${BASE_URL}/Logomark.png`,
  links: {
    docs: "https://docs.planoai.dev",
    github: "https://github.com/katanemo/plano",
    discord: "https://discord.gg/pGZf2gcwEc",
    huggingface: "https://huggingface.co/katanemo",
  },
  keywords: [
    // High-intent comparison/alternative searches (proven search volume)
    "LiteLLM alternative",
    "Portkey alternative",
    "Helicone alternative",
    "OpenRouter alternative",
    "Kong AI Gateway alternative",

    // Primary keywords (high volume, validated by industry reports)
    "AI gateway",
    "LLM gateway",
    "agentic AI",
    "AI agents",
    "agent orchestration",
    "LLM routing",

    // MCP - massive 2025 trend (97M+ SDK downloads, industry standard)
    "MCP server",
    "Model Context Protocol",
    "MCP gateway",
    "MCP observability",
    "MCP security",

    // Problem-aware searches (how developers search)
    "LLM rate limiting",
    "LLM load balancing",
    "LLM failover",
    "provider fallback",
    "multi-provider LLM",
    "LLM cost optimization",
    "token usage tracking",

    // Agent framework integration (trending frameworks)
    "LangGraph gateway",
    "LangChain infrastructure",
    "CrewAI deployment",
    "AutoGen orchestration",
    "multi-agent orchestration",

    // Production & reliability (enterprise focus)
    "AI agents in production",
    "production AI infrastructure",
    "agent reliability",
    "deploy AI agents",
    "scaling AI agents",
    "LLM traffic management",

    // Observability & LLMOps (growing category)
    "LLM observability",
    "AI observability",
    "agent tracing",
    "LLMOps",
    "AI telemetry",
    "prompt versioning",

    // Guardrails & safety (enterprise requirement)
    "AI guardrails",
    "LLM content filtering",
    "prompt injection protection",
    "AI safety middleware",

    // Routing & optimization
    "model routing",
    "inference routing",
    "latency based routing",
    "intelligent model selection",
    "semantic caching LLM",

    // Emerging trends (A2A, agentic RAG)
    "A2A protocol",
    "agent to agent communication",
    "agentic RAG",
    "tool calling orchestration",
    "function calling routing",

    // Use cases (specific applications)
    "RAG infrastructure",
    "chatbot backend",
    "AI customer service infrastructure",
    "coding agent infrastructure",

    // Infrastructure architecture
    "AI data plane",
    "AI control plane",
    "AI proxy",
    "unified LLM API",

    // Open source & self-hosted (strong developer interest)
    "open source AI gateway",
    "open source LLM gateway",
    "self hosted AI gateway",
    "on premise LLM routing",

    // Brand (minimal, necessary)
    "Plano AI",
    "Plano gateway",
  ],
  authors: [{ name: "Katanemo", url: "https://github.com/katanemo/plano" }],
  creator: "Katanemo",
};

/**
 * Generate page-specific metadata with consistent defaults
 */
export function createMetadata({
  title,
  description,
  keywords = [],
  image,
  noIndex = false,
  pathname = "",
}: {
  title?: string;
  description?: string;
  keywords?: string[];
  image?: string;
  noIndex?: boolean;
  pathname?: string;
}): Metadata {
  const pageTitle = title
    ? `${title} | ${siteConfig.name}`
    : `${siteConfig.name} - ${siteConfig.tagline}`;

  const pageDescription = description || siteConfig.description;
  const pageImage = image || siteConfig.ogImage;
  const pageUrl = pathname ? `${BASE_URL}${pathname}` : BASE_URL;

  return {
    title: pageTitle,
    description: pageDescription,
    keywords: [...siteConfig.keywords, ...keywords],
    authors: siteConfig.authors,
    creator: siteConfig.creator,
    metadataBase: new URL(BASE_URL),
    alternates: {
      canonical: pageUrl,
    },
    openGraph: {
      type: "website",
      locale: "en_US",
      url: pageUrl,
      title: pageTitle,
      description: pageDescription,
      siteName: siteConfig.name,
      images: [
        {
          url: pageImage,
          width: 1200,
          height: 630,
          alt: `${siteConfig.name} - ${siteConfig.tagline}`,
        },
      ],
    },
    twitter: {
      card: "summary_large_image",
      title: pageTitle,
      description: pageDescription,
      images: [pageImage],
      creator: "@katanemo",
    },
    robots: noIndex
      ? {
          index: false,
          follow: false,
        }
      : {
          index: true,
          follow: true,
          googleBot: {
            index: true,
            follow: true,
            "max-video-preview": -1,
            "max-image-preview": "large",
            "max-snippet": -1,
          },
        },
  };
}

/**
 * Default metadata for the root layout
 */
export const defaultMetadata: Metadata = createMetadata({});

/**
 * Page-specific metadata configurations
 */
export const pageMetadata = {
  home: createMetadata({
    pathname: "/",
    keywords: ["AI gateway", "agent orchestration", "LLM routing"],
  }),

  research: createMetadata({
    title: "Research",
    description:
      "Explore Plano's applied AI research focusing on safe and efficient agent delivery. Discover our orchestrator models, benchmarks, and open-source LLMs on Hugging Face.",
    pathname: "/research",
    keywords: [
      "AI research",
      "orchestrator models",
      "Plano orchestrator",
      "AI benchmarks",
      "open source LLM",
    ],
  }),

  blog: createMetadata({
    title: "Blog",
    description:
      "Latest insights, tutorials, and updates from Plano. Learn about AI agents, agent orchestration, LLM routing, and building production-ready agentic applications.",
    pathname: "/blog",
    keywords: [
      "AI blog",
      "agent tutorials",
      "LLM guides",
      "AI engineering",
      "agentic AI",
      "Plano blog",
      "Plano blog posts",
      "Plano gateway blog",
    ],
  }),

  contact: createMetadata({
    title: "Contact",
    description:
      "Get in touch with the Plano team. Join our Discord community or contact us for enterprise solutions for your AI agent infrastructure needs.",
    pathname: "/contact",
    keywords: ["contact Plano", "AI support", "enterprise AI", "AI consulting"],
  }),

  docs: createMetadata({
    title: "Documentation",
    description:
      "Comprehensive documentation for Plano. Learn how to set up agent orchestration, LLM routing, guardrails, and observability for your AI applications.",
    pathname: "/docs",
    keywords: [
      "Plano docs",
      "AI gateway documentation",
      "agent setup guide",
      "LLM configuration",
    ],
  }),
};

/**
 * Generate metadata for blog posts
 */
export function createBlogPostMetadata({
  title,
  description,
  slug,
  publishedAt,
  author,
  image,
}: {
  title: string;
  description?: string;
  slug: string;
  publishedAt?: string;
  author?: string;
  image?: string;
}): Metadata {
  const pageUrl = `${BASE_URL}/blog/${slug}`;
  // Use the dynamic OG image endpoint for blog posts
  const ogImage = `${BASE_URL}/api/og/${slug}`;

  return {
    title: `${title} | ${siteConfig.name} Blog`,
    description:
      description ||
      `Read "${title}" on the Plano blog. Insights about AI agents, orchestration, and building production-ready agentic applications.`,
    authors: author ? [{ name: author }] : siteConfig.authors,
    metadataBase: new URL(BASE_URL),
    alternates: {
      canonical: pageUrl,
    },
    openGraph: {
      type: "article",
      locale: "en_US",
      url: pageUrl,
      title: title,
      description: description || `Read "${title}" on the Plano blog.`,
      siteName: siteConfig.name,
      publishedTime: publishedAt,
      authors: author ? [author] : undefined,
      images: [
        {
          url: ogImage,
          width: 1200,
          height: 630,
          alt: title,
        },
      ],
    },
    twitter: {
      card: "summary_large_image",
      title: title,
      description: description || `Read "${title}" on the Plano blog.`,
      images: [ogImage],
      creator: "@katanemo",
    },
    robots: {
      index: true,
      follow: true,
      googleBot: {
        index: true,
        follow: true,
        "max-video-preview": -1,
        "max-image-preview": "large",
        "max-snippet": -1,
      },
    },
  };
}
