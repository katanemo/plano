import type { Metadata } from "next";
import { pageMetadata } from "@/lib/metadata";
import ResearchPageClient from "./ResearchPageClient";

export const metadata: Metadata = pageMetadata.research;

export default function ResearchPage() {
  return <ResearchPageClient />;
}
