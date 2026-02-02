import { MetadataRoute } from "next";

const BASE_URL = process.env.NEXT_PUBLIC_APP_URL || "https://planoai.dev";

export default function robots(): MetadataRoute.Robots {
  return {
    rules: [
      {
        userAgent: "*",
        allow: "/",
        disallow: [
          "/studio/", // Sanity Studio admin
          "/api/", // API routes
          "/_next/", // Next.js internal routes
        ],
      },
      {
        // Specific rules for common crawlers
        userAgent: "Googlebot",
        allow: "/",
        disallow: ["/studio/", "/api/"],
      },
      {
        userAgent: "Bingbot",
        allow: "/",
        disallow: ["/studio/", "/api/"],
      },
    ],
    sitemap: `${BASE_URL}/sitemap.xml`,
    host: BASE_URL,
  };
}
