import React from "react";
import Link from "next/link";
import Image from "next/image";

const footerLinks = {
  product: [
    { label: "Research", href: "/research" },
    { label: "Blog", href: "/blog" },
    { label: "Documentation", href: "https://docs.planoai.dev", external: true },
    { label: "Hugging Face", href: "https://huggingface.co/katanemo", external: true },
  ],
  resources: [
    { label: "GitHub", href: "https://github.com/katanemo/archgw", external: true },
    { label: "Discord", href: "https://discord.gg/pGZf2gcwEc", external: true },
    { label: "Get Started", href: "https://docs.planoai.dev/get_started/installation", external: true },
  ],
};

export function Footer() {
  return (
    <footer
      className="relative overflow-hidden pt-20 px-6 lg:px-[102px] pb-48"
      style={{ background: "linear-gradient(to top right, #ffffff, #dcdfff)" }}
    >
      <div className="max-w-[81rem] mx-auto relative z-10">
        {/* Main Grid Layout */}
        <div className="grid grid-cols-1 lg:grid-cols-2 gap-20">
          {/* Left Column - Tagline and Copyright */}
          <div className="flex flex-col">
            <p className="ext-base sm:text-lg md:text-xl lg:text-[22px] font-sans font-normal tracking-[-1.0px] sm:tracking-[-1.22px]! text-black mb-6 sm:mb-8">
            Plano is a powerful agent delivery infrastructure platform that is framework-friendly,
            and empowers developers and teams to seamlessly build, deliver, and scale agentic
            applications.
            </p>

            {/* Copyright */}
            <div className="mt-auto">
              <p className="font-sans text-sm sm:text-base text-black/63 tracking-[-0.6px] sm:tracking-[-0.8px]!">
                © 2026 Katanemo Labs, a DigitalOcean Company / Plano by Katanemo Labs
              </p>
            </div>
          </div>

          {/* Right Column - Navigation Links */}
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-8">
            {/* Product Links */}
            <div>
              <h3 className="font-sans font-normal text-xl sm:text-2xl lg:text-3xl text-black tracking-[-1.2px] sm:tracking-[-1.4px] lg:tracking-[-1.6px]! mb-4 sm:mb-6">
                Product
              </h3>
              <nav className="space-y-3 sm:space-y-4">
                {footerLinks.product.map((link) => (
                  <Link
                    key={link.href}
                    href={link.href}
                    target={link.external ? "_blank" : undefined}
                    rel={link.external ? "noopener noreferrer" : undefined}
                    className="block font-sans font-normal text-sm sm:text-base lg:text-lg text-black tracking-[-0.8px] sm:tracking-[-0.9px] lg:tracking-[-1px]! hover:text-[var(--primary)] transition-colors"
                  >
                    {link.label}
                  </Link>
                ))}
              </nav>
            </div>

            {/* Resources Links */}
            <div>
              <h3 className="font-sans font-normal text-xl sm:text-2xl lg:text-3xl text-black tracking-[-1.2px] sm:tracking-[-1.4px] lg:tracking-[-1.6px]! mb-4 sm:mb-6">
                Resources
              </h3>
              <nav className="space-y-3 sm:space-y-4">
                {footerLinks.resources.map((link) => (
                  <Link
                    key={link.href}
                    href={link.href}
                    target={link.external ? "_blank" : undefined}
                    rel={link.external ? "noopener noreferrer" : undefined}
                    className="block font-sans font-normal text-sm sm:text-base lg:text-lg text-black tracking-[-0.8px] sm:tracking-[-0.9px] lg:tracking-[-1px]! hover:text-[var(--primary)] transition-colors"
                  >
                    {link.label}
                  </Link>
                ))}
              </nav>
            </div>
          </div>
        </div>
      </div>

      {/* Half-Cut Plano Logo Background */}
      <div className="absolute bottom-0 left-0 right-0 overflow-hidden pointer-events-none">
        <div className="max-w-[81rem] mx-auto px-6 lg:px-[1px]">
          <div className="relative w-full flex justify-start">
            <Image
              src="/LogoOutline.svg"
              alt="Plano Logo"
              width={1800}
              height={200}
              className="w-150 h-auto opacity-30 select-none"
              style={{
                transform: "translateY(0%)", // Push logo down more while showing top part
                transformOrigin: "center bottom",
              }}
            />
          </div>
        </div>
      </div>
    </footer>
  );
}
