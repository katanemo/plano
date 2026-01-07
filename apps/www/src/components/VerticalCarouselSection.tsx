"use client";

import React, { useState } from "react";
import Image from "next/image";
import { motion, AnimatePresence } from "framer-motion";
import { Button } from "@katanemo/ui";

const verticalCarouselData = [
  {
    id: 1,
    category: "INTRODUCTION",
    title: "",
    description: [
      "Plano is an AI-native data plane for AI agents - a framework-friendly, protocol-native fabric that lets you focus on what really matters: your agents' product logic.",
      "Plano takes over the plumbing work that slows teams down when handling and processing prompts, including detecting and blocking jailbreaks, routing tasks to the right model or agent for better accuracy, applying context engineering hooks, and centralizing observability across agentic interactions.",
    ],
    diagram: "/IntroDiagram.svg",
  },
  {
    id: 2,
    category: "OPEN SOURCE",
    title: "",
    description: [
      "No lock-in. No black boxes. Just an open, intelligent fabric for building more reliable agentic AI applications.",
      "Built by engineers with roots in the Envoy ecosystem, Plano brings production-grade reliability to agent traffic and prompt orchestration—while staying fully extensible. Shape it, extend it, and integrate it into your existing workflows without being forced into a rigid framework or a single provider.",
    ],
    diagram: "/OpenSource.svg",
  },
  {
    id: 3,
    category: "BUILT ON ENVOY",
    title: "",
    description: [
      "Plano is built on Envoy and runs as a self-contained sidecar alongside your application servers. It extends Envoy's HTTP connection management, filtering, and telemetry specifically for prompt and LLM traffic—so you get production-grade routing, policy enforcement, and observability out of the box.",
      "Use Plano with any application language or framework, and connect it to any LLM provider.",
    ],
    diagram: "/BuiltOnEnvoy.svg",
  },
  {
    id: 4,
    category: "PURPOSE-BUILT",
    title: "",
    description: [
      "Unlike generic API gateways, Plano is purpose-built for agent workloads, where prompts are the unit of work.",
      "Plano treats prompts as first-class traffic: it understands prompt/response flows, tool calls, model selection, and multi-agent handoffs. That means routing, policy enforcement, and observability are optimized for agent execution—not retrofitted from traditional API infrastructure—so your AI applications stay fast, reliable, and easy to evolve.",
    ],
    diagram: "/PurposeBuilt.svg",
  },
  {
    id: 5,
    category: "PROGRAMMABLE ARCHITECTURE",
    title: "",
    description: [
      'As agent workloads move beyond prototypes, teams end up scattering critical logic across apps: compliance checks, context "patches," provider-specific quirks, etc. That glue code gets duplicated across agents, is hard to audit, and slows iteration because every policy or workflow change requires touching application code and redeploying.',
      "Plano keeps that logic in one place with a programmable Agent Filter Chain—hooks that can inspect, mutate, or terminate prompt traffic early, turning common steps (policy enforcement, jailbreak checks, context engineering, tool gating, routing hints) into reusable building blocks.",
    ],
    diagram: "/PromptRouting.svg",
  },
];

export function VerticalCarouselSection() {
  const [activeSlide, setActiveSlide] = useState(0);

  const handleSlideClick = (index: number) => {
    setActiveSlide(index);
  };

  return (
    <section className="relative bg-[#1a1a1a] text-white pt-20 pb-0 lg:pb-4 px-4 sm:px-6 lg:px-[102px] h-auto sm:h-[650px]">
      <div className="max-w-324 mx-auto">
        {/* Main Heading */}
        <h2 className="font-sans font-normal text-2xl sm:text-3xl lg:text-4xl tracking-[-2px] sm:tracking-[-2.88px]! text-white leading-[1.03] mb-8 sm:mb-12 lg:mb-12 max-w-4xl">
          Under the hood
        </h2>

        {/* Mobile: Horizontal Scroller Navigation */}
        <div className="lg:hidden mb-8 -mx-4 sm:mx-0 px-4 sm:px-0">
          <div
            className="relative overflow-x-auto pb-2"
            style={{
              scrollbarWidth: "none",
              msOverflowStyle: "none",
            }}
          >
            <style jsx>{`
              .hide-scrollbar::-webkit-scrollbar {
                display: none;
              }
            `}</style>
            <div className="flex gap-4 min-w-max hide-scrollbar">
              {verticalCarouselData.map((item, index) => (
                <button
                  key={item.id}
                  onClick={() => handleSlideClick(index)}
                  className={`relative px-4 py-2 rounded transition-all duration-300 whitespace-nowrap ${
                    index === activeSlide
                      ? "bg-[#6363d2]/90 text-[#f9faff]"
                      : "bg-[#6363d2]/10 text-[rgba(182,188,255,0.71)] hover:bg-[#6363d2]/15"
                  }`}
                >
                  <span className="font-mono font-bold text-sm tracking-[1.44px]!">
                    {item.category}
                  </span>
                </button>
              ))}
            </div>
          </div>
        </div>

        {/* Desktop: Vertical Carousel Layout */}
        <div className="flex flex-col lg:flex-row lg:items-start">
          {/* Left Sidebar Navigation - Desktop Only */}
          <div className="hidden lg:block lg:w-72 shrink-0 lg:pt-0">
            <div className="relative space-y-6">
              <motion.div
                className="absolute left-0 top-0 w-2 h-4 bg-[#6363d2] z-10 rounded-xs"
                animate={{
                  y: activeSlide * 52 + 6, // Each item is ~28px text + 24px gap = 52px, +10px to center smaller rectangle
                }}
                transition={{
                  type: "spring",
                  stiffness: 300,
                  damping: 30,
                  duration: 0.6,
                }}
              />

              {verticalCarouselData.map((item, index) => (
                <div
                  key={item.id}
                  onClick={() => handleSlideClick(index)}
                  className="cursor-pointer relative pl-6 transition-all duration-300"
                >
                  {/* Category Text */}
                  <span
                    className={`font-mono font-bold text-lg tracking-[1.69px]! transition-colors duration-300 ${
                      index === activeSlide
                        ? "text-[#acb3fe]"
                        : "text-[rgba(172,179,254,0.71)]"
                    }`}
                  >
                    {item.category}
                  </span>
                </div>
              ))}
            </div>
          </div>

          {/* Right Content Area - Fixed height to prevent layout shift */}
          <div className="flex-1 h-[600px] sm:h-[650px] lg:h-[600px] relative lg:-ml-8">
            <AnimatePresence mode="wait">
              <motion.div
                key={activeSlide}
                initial={{ opacity: 0, y: 20 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: -20 }}
                transition={{ duration: 0.4, ease: "easeInOut" }}
                className="w-full h-full"
              >
                <div className="flex flex-col lg:flex-row gap-6 sm:gap-8 lg:gap-12 items-start h-full">
                  {/* Diagram - Above on mobile, Right Side on desktop */}
                  <div className="w-full lg:flex-1 flex items-center justify-center lg:justify-start order-first lg:order-last shrink-0">
                    <div className="relative w-full max-w-full sm:max-w-md lg:max-w-[600px] aspect-4/3">
                      <Image
                        src={verticalCarouselData[activeSlide].diagram}
                        alt={verticalCarouselData[activeSlide].category}
                        fill
                        className="object-contain object-top"
                        priority
                      />
                    </div>
                  </div>

                  {/* Text Content */}
                  <div className="flex-1 max-w-2xl order-last lg:order-first flex flex-col justify-start">
                    {/* Title
                    <h3 className="font-sans font-medium text-primary text-xl sm:text-2xl lg:text-[34px] tracking-[-1px]! leading-[1.03] mb-4 sm:mb-6">
                      {verticalCarouselData[activeSlide].title}
                    </h3> */}

                    {/* Description */}
                    <div className="text-white text-sm sm:text-base lg:text-lg max-w-full lg:max-w-md -mt-0.5">
                      {verticalCarouselData[activeSlide].description.map(
                        (paragraph, index) => (
                          <p
                            key={index}
                            className={
                              index <
                              verticalCarouselData[activeSlide].description
                                .length -
                                1
                                ? "mb-4"
                                : "mb-0"
                            }
                          >
                            {paragraph}
                          </p>
                        ),
                      )}
                    </div>
                  </div>
                </div>
              </motion.div>
            </AnimatePresence>
          </div>
        </div>
      </div>
    </section>
  );
}
