"use client";

import React, { useState, useEffect } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { Button } from "@katanemo/ui";
import Link from "next/link";

const carouselData = [
  {
    id: 1,
    category: "LAUNCH FASTER",
    title: "Focus on core objectives",
    description:
      "Building agents is hard enough. The plumbing work shouldn't be. Plano handles routing, observability, and policy hooks as an AI-native sidecar—so you can focus on your agent's core product logic and ship to production faster.",
    image: "/LaunchFaster.svg",
    link: "https://docs.planoai.dev/get_started/quickstart",
  },
  {
    id: 2,
    category: "BUILD WITH CHOICE",
    title: "Rapidly incorporate LLMs",
    description:
      "Build with multiple LLMs or model versions with a single unified API. Plano centralizes access controls, offers resiliency for traffic to 100+ LLMs -- all without you having to write a single line of code. Use existing libraries and proxy traffic through Plano.",
    image: "/BuildWithChoice.svg",
    link: "https://docs.planoai.dev/concepts/llm_providers/llm_providers",
  },
  {
    id: 3,
    category: "RICH LEARNING SIGNALS",
    title: "Hyper-rich agent traces and logs",
    description:
      "Knowing when agents fail or delight users is a critical signal that feeds into the reinforcement learning and optimization cycle. Plano makes this trivial by sampling hyper-rich information traces from live production agentic interactions so that you can improve agent performance faster.",
    image: "/Telemetry.svg",
    link: "https://docs.planoai.dev/guides/observability/observability.html",
  },
  {
    id: 4,
    category: "SHIP CONFIDENTLY",
    title: "Centrally apply guardrail policies",
    description:
      "Plano comes built-in with a state-of-the-art guardrail model you can use for things like jailbreak detection. But you can easily extend those capabilities via plano's agent filter chain to apply custom policy checks in a centralized way and keep users engaged on topics relevant to your requirements.",
    image: "/ShipConfidently.svg",
    link: "https://docs.planoai.dev/guides/prompt_guard.html",
  },
  {
    id: 5,
    category: "SCALABLE ARCHITECTURE",
    title: "Protocol-Native Infrastructure",
    description:
      "Plano's sidecar deployment model avoids library-based abstractions - operating as a protocol-native data plane that integrates seamlessly with your existing agents via agentic APIs (like v1/responses). This decouples your core agent logic from plumbing concerns - run it alongside any framework without code changes, vendor lock-in, or performance overhead.",
    image: "/Contextual.svg",
    link: "https://docs.planoai.dev/concepts/tech_overview/tech_overview.html",
  },
];

export function IdeaToAgentSection() {
  const [currentSlide, setCurrentSlide] = useState(0);
  const [isAutoPlaying, setIsAutoPlaying] = useState(true);

  // Auto-advance slides
  useEffect(() => {
    if (!isAutoPlaying) return;

    const interval = setInterval(() => {
      setCurrentSlide((prev) => (prev + 1) % carouselData.length);
    }, 10000); // 10 seconds per slide

    return () => clearInterval(interval);
  }, [isAutoPlaying]);

  const handleSlideClick = (index: number) => {
    setCurrentSlide(index);
    setIsAutoPlaying(false);
    // Resume auto-play after 10 seconds
    setTimeout(() => setIsAutoPlaying(true), 10000);
  };

  return (
    <section className="relative py-12 sm:py-16 lg:py-24 px-4 sm:px-6 lg:px-[102px]">
      <div className="max-w-[81rem] mx-auto">
        {/* Main Heading */}
        <h2 className="font-sans font-normal text-2xl sm:text-3xl lg:text-4xl tracking-[-2px] sm:tracking-[-2.96px]! text-black mb-6 sm:mb-8 lg:mb-10">
          Idea to agent — without overhead
        </h2>

        {/* Progress Indicators */}
        <div className="flex gap-1.5 sm:gap-2 mb-4 sm:mb-6 lg:mb-6 w-full">
          {carouselData.map((_, index) => (
            <button
              key={index}
              onClick={() => handleSlideClick(index)}
              className={`relative h-1.5 sm:h-2 rounded-full overflow-hidden transition-all duration-300 hover:opacity-80 ${
                index === currentSlide
                  ? "flex-1 sm:w-16 md:w-20 lg:w-[292px]"
                  : "flex-1 sm:w-16 md:w-20 lg:w-[293px]"
              }`}
            >
              {/* Background */}
              <div className="absolute inset-0 bg-black/6 rounded-full" />

              {/* Active Progress */}
              {index === currentSlide && (
                <motion.div
                  className="absolute inset-0 bg-[#7780d9] rounded-full"
                  initial={{ width: 0 }}
                  animate={{ width: "100%" }}
                  transition={{ duration: 10, ease: "linear" }}
                  key={currentSlide}
                />
              )}

              {/* Completed State */}
              {index < currentSlide && (
                <div className="absolute inset-0 bg-purple-200/90 rounded-full" />
              )}
            </button>
          ))}
        </div>

        {/* Carousel Content - Fixed height to prevent layout shift */}
        <div className="relative h-[500px] sm:h-[550px] md:h-[600px] lg:h-[500px]">
          <AnimatePresence mode="wait">
            <motion.div
              key={currentSlide}
              initial={{ opacity: 0, y: 20 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -20 }}
              transition={{ duration: 0.4, ease: "easeInOut" }}
              className="absolute inset-0"
            >
              <div className="flex flex-col lg:flex-row lg:justify-between lg:items-center lg:gap-12 h-full">
                {/* Left Content */}
                <div className="flex-1 order-1 lg:order-1 flex flex-col justify-center">
                  <div className="max-w-[692px] mt-0 lg:mt-0">
                    {/* Category */}
                    <p className="font-mono font-bold text-[#2a3178] text-sm sm:text-base lg:text-xl tracking-[1.44px] sm:tracking-[1.92px]! mb-3 sm:mb-4 leading-[1.102]">
                      {carouselData[currentSlide].category}
                    </p>

                    {/* Title */}
                    <h3 className="font-sans font-medium text-[#9797ea] text-2xl sm:text-3xl lg:text-5xl tracking-tight sm:tracking-[-2.96px]! mb-4 sm:mb-6 lg:mb-7">
                      {carouselData[currentSlide].title}
                    </h3>

                    {/* Description */}
                    <div className="text-black text-sm sm:text-base lg:text-lg max-w-full lg:max-w-140">
                      <p className="mb-0">
                        {carouselData[currentSlide].description}
                      </p>
                    </div>

                    <Button asChild className="mt-6 sm:mt-8 w-full sm:w-auto">
                      <Link href={carouselData[currentSlide].link}>
                        Learn more
                      </Link>
                    </Button>
                  </div>
                </div>

                {/* Image - Show below on mobile, right side on desktop */}
                {carouselData[currentSlide].image && (
                  <div className="flex lg:hidden shrink-0 w-full justify-center items-center mb-6 sm:mb-8 order-0 lg:order-2">
                    <img
                      src={carouselData[currentSlide].image}
                      alt={carouselData[currentSlide].category}
                      className={`w-full h-auto object-contain ${
                        carouselData[currentSlide].image === "/Telemetry.svg"
                          ? "max-w-md sm:max-w-lg max-h-[300px] sm:max-h-[350px]"
                          : "max-w-sm sm:max-w-md max-h-[250px] sm:max-h-[300px]"
                      }`}
                    />
                  </div>
                )}

                {/* Right Image - Desktop only */}
                {carouselData[currentSlide].image && (
                  <div
                    className={`hidden lg:flex shrink-0 justify-end items-center order-2 ${
                      carouselData[currentSlide].image === "/Telemetry.svg"
                        ? "w-[500px] xl:w-[600px]"
                        : "w-[400px] xl:w-[500px]"
                    }`}
                  >
                    <img
                      src={carouselData[currentSlide].image}
                      alt={carouselData[currentSlide].category}
                      className={`w-full h-auto object-contain ${
                        carouselData[currentSlide].image === "/Telemetry.svg"
                          ? "max-h-[550px]"
                          : "max-h-[450px]"
                      }`}
                    />
                  </div>
                )}
              </div>
            </motion.div>
          </AnimatePresence>
        </div>
      </div>
    </section>
  );
}
