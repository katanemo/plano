import { createClient } from "@sanity/client";
import imageUrlBuilder from "@sanity/image-url";
import type { SanityImageSource } from "@sanity/image-url/lib/types/types";

const projectId =
  process.env.NEXT_PUBLIC_SANITY_PROJECT_ID ||
  "71ny25bn";
const dataset =
  process.env.NEXT_PUBLIC_SANITY_DATASET ||
  "production";
const apiVersion =
  process.env.NEXT_PUBLIC_SANITY_API_VERSION ||
  "2025-01-01";

export const hasSanityConfig = Boolean(projectId && dataset && apiVersion);

export const client = hasSanityConfig
  ? createClient({
      projectId,
      dataset,
      apiVersion,
      // Keep blog/admin updates visible immediately after publishing.
      useCdn: false,
    })
  : null;

const builder = client ? imageUrlBuilder(client) : null;

export function urlFor(source: SanityImageSource) {
  if (!builder) {
    throw new Error("Sanity client is not configured.");
  }
  return builder.image(source);
}
