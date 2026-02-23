import React from "react";
import { Button } from "@katanemo/ui";
import Link from "next/link";
import { NetworkAnimation } from "./NetworkAnimation";

export function Hero() {
  return (
    <section className="relative pt-8 sm:pt-12 lg:pt-1 pb-6 px-4 sm:px-6 lg:px-8 overflow-hidden">
      <div className="max-w-[81rem] mx-auto relative">
        <div className="hidden lg:block absolute inset-0 pointer-events-none ">
          <NetworkAnimation />
        </div>
        <div className="lg:hidden absolute inset-0 pointer-events-none">
          <NetworkAnimation className="!w-[300px] !h-[300px] left-82! top-1! opacity-90! " />
        </div>
        <div className="max-w-3xl mb-3 sm:mb-4 relative z-10">
          {/* Version Badge */}
          <div className="mb-4 sm:mb-6">
            <Link
              href="https://docs.planoai.dev/concepts/signals.html"
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex"
            >
              <div className="inline-flex flex-wrap items-center gap-1.5 sm:gap-2 px-3 sm:px-4 py-1 rounded-full bg-[rgba(185,191,255,0.4)] border border-[var(--secondary)] shadow backdrop-blur hover:bg-[rgba(185,191,255,0.6)] transition-colors cursor-pointer">
                <span className="text-xs sm:text-sm font-medium text-black/65">
                  v0.4.8
                </span>
                <span className="text-xs sm:text-sm font-medium text-black ">
                  —
                </span>
                <span className="text-xs sm:text-sm font-[600] tracking-[-0.6px]! text-black leading-tight">
                  <span className="hidden sm:inline">
                    Signals: Trace Sampling for Fast Error Analysis
                  </span>
                  <span className="sm:hidden">
                    Signals: Trace Sampling for Fast Error Analysis
                  </span>
                </span>
              </div>
            </Link>
          </div>

          {/* Main Heading */}
          <h1 className="text-4xl sm:text-4xl md:text-5xl lg:text-7xl font-normal leading-tight tracking-tighter text-black flex flex-col gap-0 sm:-space-y-2 lg:-space-y-3">
            <span className="font-sans">Delivery Infrastructure </span>
            <span className="font-sans font-medium text-[var(--secondary)]">
              for Agentic Apps
            </span>
          </h1>
        </div>

        {/* Subheading with CTA Buttons */}
        <div className="max-w-7xl relative z-10">
          <p className="text-base sm:text-lg md:text-xl lg:text-[22px] font-sans font-[400] tracking-[-1.0px] sm:tracking-[-1.22px]! text-black max-w-76 sm:max-w-2xl mb-6">
            Build agents faster, and deliver them reliably to production - by
            offloading the critical plumbing work to Plano.
          </p>

          {/* CTA Buttons */}
          <div className="flex flex-col sm:flex-row items-stretch sm:items-start gap-3 sm:gap-4">
            <Button asChild className="w-full sm:w-auto">
              <Link
                href="https://docs.planoai.dev/get_started/quickstart"
                target="_blank"
                rel="noopener noreferrer"
              >
                Get started
              </Link>
            </Button>
            <Button variant="secondary" asChild className="w-full sm:w-auto">
              <Link
                href="https://docs.planoai.dev"
                target="_blank"
                rel="noopener noreferrer"
              >
                Documentation
              </Link>
            </Button>
          </div>
        </div>
      </div>
    </section>
  );
}
