import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  experimental: {
    externalDir: true,
  },
  webpack: (config) => {
    config.resolve.modules = [
      ...(config.resolve.modules || []),
      "node_modules",
      "../../node_modules",
    ];
    return config;
  },
  turbopack: {
    resolveAlias: {},
  },
};

export default nextConfig;
