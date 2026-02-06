import path from "node:path";
import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  transpilePackages: [
    "@katanemo/ui",
    "@katanemo/shared-styles",
    "@katanemo/tailwind-config",
    "@katanemo/tsconfig",
  ],
  experimental: {
    // Ensure workspace packages are handled correctly
    externalDir: true,
  },
  // Webpack config for production builds
  webpack: (config, { isServer }) => {
    // Ensure proper resolution of dependencies in monorepo
    config.resolve.modules = [
      ...(config.resolve.modules || []),
      "node_modules",
      "../../node_modules",
    ];

    if (!isServer) {
      config.resolve.fallback = {
        ...config.resolve.fallback,
        fs: false,
      };
    }
    return config;
  },
  // Turbopack config for dev mode (Next.js 16 default)
  turbopack: {
    root: path.join(__dirname, "../.."), // Monorepo root so Next.js doesn't warn about multiple lockfiles
    resolveAlias: {
      // Turbopack should handle monorepo resolution automatically
      // but we can add specific aliases if needed
    },
  },
  images: {
    remotePatterns: [
      {
        protocol: "https",
        hostname: "cdn.sanity.io",
        port: "",
        pathname: "/**",
      },
    ],
  },
};

export default nextConfig;
