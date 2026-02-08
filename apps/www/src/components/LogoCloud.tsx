import React from "react";
import Image from "next/image";

const customerLogos = [
  {
    name: "HuggingFace",
    src: "/logos/huggingface.svg",
  },
  {
    name: "T-Mobile",
    src: "/logos/tmobile.svg",
  },
  {
    name: "HP",
    src: "/logos/hp.svg",
  },
  {
    name: "SanDisk",
    src: "/logos/sandisk.svg",
  },
  {
    name: "ClubCentric",
    src: "/logos/clubcentric.svg",
  },
];

export function LogoCloud() {
  return (
    <section className="relative py-6 sm:py-8 px-4 sm:px-6 lg:px-8 bg-transparent">
      <div className="max-w-[81rem] mx-auto">
        <div className="grid grid-cols-2 md:grid-cols-3 lg:flex lg:flex-row lg:justify-center lg:items-center gap-4 sm:gap-6 md:gap-8 lg:gap-0 place-items-center">
          {customerLogos.map((logo, index) => {
            const isLast = index === customerLogos.length - 1;
            const isTMobile = index === 1; // T-Mobile is before HP
            const isHP = index === 2; // HP is in center
            const isSanDisk = index === 3; // SanDisk is after HP
            const isClubCentric = index === 4; // ClubCentric is after SanDisk

            // Custom spacing for logos around HP on large screens
            let spacingClass = "lg:mx-6 xl:mx-8"; // Default spacing
            if (isTMobile) {
              spacingClass = "lg:mr-3 xl:mr-4 lg:ml-6 xl:ml-8"; // Smaller gap to HP
            } else if (isHP) {
              spacingClass = "lg:mx-3 xl:mx-4"; // Smaller gaps on both sides
            } else if (isSanDisk) {
              spacingClass = "lg:ml-3 xl:ml-5 lg:mr-6 xl:mr-14"; // Smaller gap from HP
            } else if (isClubCentric) {
              spacingClass = "lg:mx-3 xl:mx-4 mb-1"; // Smaller gaps on both sides
            }

            return (
              <div
                key={logo.name}
                className={`flex items-center justify-center opacity-60 hover:opacity-80 transition-opacity duration-300 w-full max-w-32 sm:max-w-40 md:max-w-48 h-10 sm:h-12 md:h-16 mx-auto ${spacingClass} ${
                  isLast ? "col-span-2 md:col-span-3 lg:col-span-none" : ""
                }`}
              >
                <Image
                  src={logo.src}
                  alt={`${logo.name} logo`}
                  width={128}
                  height={40}
                  className="w-full h-full object-contain filter grayscale hover:grayscale-0 transition-all duration-300"
                />
              </div>
            );
          })}
        </div>
      </div>
    </section>
  );
}
