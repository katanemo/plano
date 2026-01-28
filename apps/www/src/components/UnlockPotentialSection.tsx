import React from "react";
import { Button } from "@katanemo/ui";
import Link from "next/link";

interface UnlockPotentialSectionProps {
  variant?: "transparent" | "black";
  className?: string;
}

export function UnlockPotentialSection({
  variant = "transparent",
  className = "",
}: UnlockPotentialSectionProps) {
  const backgroundClass = variant === "black" ? "bg-[#1a1a1a]" : "";
  const textColor = variant === "black" ? "text-white" : "text-black";

  return (
    <section
      className={`relative py-24 px-6 lg:px-[102px]`}
      style={{ background: "linear-gradient(to top right, #ffffff, #dcdfff)" }}
    >
      <div className="max-w-[81rem] mx-auto">
        <div className="max-w-4xl">
          <h2
            className={`font-sans font-normal text-[1.8rem] lg:text-4xl tracking-[-2.55px]! ${textColor} leading-[1.4] mb-8`}
          >
            Focus on prompting, not plumbing.
            <br />
            Build with{" "}
            <strong className="font-medium text-primary">plano</strong>, get
            started in less than a minute.
          </h2>

          <div className="flex flex-col sm:flex-row gap-5">
            <Button asChild>
              <Link href="https://docs.planoai.dev/get_started/quickstart">
                Deploy today
              </Link>
            </Button>
            <Button variant="secondaryDark" asChild>
              <Link href="https://docs.planoai.dev/get_started/quickstart">
                Documentation
              </Link>
            </Button>
          </div>
        </div>
      </div>
    </section>
  );
}
