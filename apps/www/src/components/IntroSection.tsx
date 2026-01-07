import React from "react";
import Image from "next/image";

export function IntroSection() {
  return (
    <section className="relative bg-[#1a1a1a] text-white py-20 px-6 lg:px-[102px]">
      <div className="max-w-324 mx-auto">
        <div className="flex flex-col lg:flex-row gap-12">
          {/* Left Content */}
          <div className="flex-1 mt-2">
            {/* Heading */}
            <p className="font-mono font-bold text-primary-light text-xl tracking-[1.92px]! mb-4 leading-[1.102]">
              WHY PLANO?
            </p>
            <h2 className="font-sans font-medium tracking-[-1.92px]! text-[#9797ea] text-4xl leading-[1.102] mb-6 max-w-[633px]">
              Deliver prototypes to production
              <span className="italic">—fast.</span>
            </h2>

            {/* Body Text */}
            <div className="text-white text-sm sm:text-base lg:text-lg max-w-[713px]">
              <p className="mb-0">
                Plano is an AI-native proxy and dataplane for agents that
                handles critical plumbing work in AI - agent routing and
                orchestration, rich agentic traces, guardrail hooks, and smart
                model routing APIs for LLMs. Use any language, AI framework, and
                deliver agents to productions quickly with Plano.
              </p>
              <p className="mb-0  mt-4">
                Developers can focus more on core product logic of agents.
                Product teams can accelerate feedback loops for reinforcement
                learning. Engineering teams can standardize policies and access
                controls across every agent and LLM for safer, more reliable
                scaling.
              </p>
            </div>
          </div>

          {/* Right Diagram */}
          <div className="flex-1 relative w-full">
            <Image
              src="/IntroDiagram.svg"
              alt="Network Path Diagram"
              width={800}
              height={600}
              className="w-full h-auto"
              priority
            />
          </div>
        </div>
      </div>
    </section>
  );
}
