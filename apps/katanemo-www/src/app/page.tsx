import Image from "next/image";
import Link from "next/link";

export default function HomePage() {
  return (
    <main className="relative flex min-h-screen items-center justify-center overflow-hidden px-6 pt-12 pb-16 font-sans sm:pt-20 lg:items-start lg:justify-start lg:pt-24">
      <div className="relative mx-auto w-full max-w-6xl flex flex-col items-center justify-center text-left lg:items-start lg:justify-start">
        <div className="pointer-events-none mb-6 w-full self-start lg:hidden">
          <Image
            src="/KatanemoLogo.svg"
            alt="Katanemo Logo"
            width={64}
            height={64}
            priority
            className="h-auto w-10 sm:w-20"
          />
        </div>
        <div className="relative z-10 max-w-xl sm:max-w-2xl lg:max-w-2xl xl:max-w-8xl lg:pr-[26vw] xl:pr-[2vw] sm:right-0 md:right-0 lg:right-0 xl:right-20 2xl:right-50 sm:mt-36 mt-0">
          <h1 className="text-3xl sm:text-4xl md:text-5xl lg:text-6xl font-sans font-medium leading-tight tracking-tight text-white">
            Forward-deployed AI infrastructure engineers.
          </h1>
          <p className="mt-4 font-light tracking-[-0.4px] max-w-2xl text-base sm:text-lg md:text-xl lg:text-2xl text-white/70">
            Bringing industry-leading research and open-source technologies to
            accelerate the development of AI agents.
          </p>
          <div className="mt-18 flex flex-col gap-3 text-lg sm:text-xl lg:text-3xl font-light tracking-wide sm:tracking-[-0.03em] leading-snug">
            <Link
              href="https://huggingface.co/katanemo"
              className="flex items-center gap-2 text-[#31C887] hover:text-[#45e394] transition-colors"
            >
              <span>Models Research</span>
              <span aria-hidden className="text-emerald-300">↗</span>
            </Link>
            <Link
              href="https://planoai.dev"
              className="flex items-center gap-2 text-[#31C887] hover:text-[#45e394] transition-colors"
            >
              <span>Plano - Open Source Agent Infrastructure</span>
              <span aria-hidden className="text-emerald-300">↗</span>
            </Link>
          </div>
          <div className="mt-24">
            <div className="sm:max-w-7xl max-w-72 mb-4 text-sm sm:text-base lg:text-lg text-white/70 tracking-[-0.3px] sm:tracking-[0.8px]! font-light">
            Move faster and more reliably by letting Katanemo do the heavy-lifting.
            </div>
            <a
              href="mailto:interest@katanemo.com"
              className="text-sm sm:text-sm text-white/50 hover:text-white transition-colors cursor-pointer"
            >
              Contact Us
            </a>
            <div className="mt-4 h-px w-52 bg-white/10" />
            <div className="mt-3 text-sm text-white/50">
              © 2026 Katanemo Labs, Inc.
            </div>
          </div>

        </div>
        <div className="pointer-events-none absolute top-50 right-[-20vw] sm:right-[-10vw] md:right-[-5vw] lg:right-[-20vw] xl:right-[-7vw] 2xl:right-[-17vw] hidden lg:block">
          <Image
            src="/KatanemoLogo.svg"
            alt="Katanemo Logo"
            width={900}
            height={1000}
            priority
            className="h-[95vh] w-auto max-w-none opacity-90"
          />
        </div>
      </div>
    </main>
  );
}
