import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  transpilePackages: [
    "@katanemo/ui",
    "@katanemo/shared-styles",
    "@katanemo/tailwind-config",
    "@katanemo/tsconfig",
  ],
  experimental: {
    externalDir: true,
  },
  webpack: (config, { isServer }) => {
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
  turbopack: {
    resolveAlias: {},
  },
};

export default nextConfig;
