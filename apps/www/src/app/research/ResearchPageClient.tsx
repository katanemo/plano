"use client";

import {
  ResearchHero,
  ResearchGrid,
  ResearchTimeline,
  ResearchCTA,
  ResearchCapabilities,
  ResearchBenchmarks,
} from "@/components/research";
import { UnlockPotentialSection } from "@/components/UnlockPotentialSection";

export default function ResearchPageClient() {
  return (
    <>
      <ResearchHero />
      <ResearchGrid />
      <ResearchTimeline />
      <ResearchCTA />
      <ResearchCapabilities />
      <ResearchBenchmarks />
      {/* <ResearchFamily /> */}
      <UnlockPotentialSection variant="transparent" />
    </>
  );
}
