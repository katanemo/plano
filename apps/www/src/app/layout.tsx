import type { Metadata } from "next";
import { ArrowRightIcon } from "@heroicons/react/16/solid";
import Link from "next/link";
import Script from "next/script";
import "@katanemo/shared-styles/globals.css";
import { Analytics } from "@vercel/analytics/next";
import { ConditionalLayout } from "@/components/ConditionalLayout";
import { defaultMetadata } from "@/lib/metadata";

export const metadata: Metadata = {
  ...defaultMetadata,
  manifest: "/manifest.json",
  icons: {
    icon: "/PlanoIcon.svg",
    apple: "/Logomark.png",
  },
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="en">
      <body className="antialiased">
        {/* Google tag (gtag.js) */}
        <Script
          src="https://www.googletagmanager.com/gtag/js?id=G-ML7B1X9HY2"
          strategy="afterInteractive"
        />
        <Script strategy="afterInteractive">
          {`
            window.dataLayer = window.dataLayer || [];
            function gtag(){dataLayer.push(arguments);}
            gtag('js', new Date());
            gtag('config', 'G-ML7B1X9HY2');
          `}
        </Script>
        <Link
          href="https://digitalocean.com"
          target="_blank"
          rel="noopener noreferrer"
          className="block w-full bg-[#7780D9] py-3 text-white transition-opacity"
        >
          <div className="mx-auto flex max-w-[85rem] items-center justify-center gap-4 px-6 text-center md:justify-between md:text-left lg:px-8">
            <span className="w-full text-xs font-medium leading-snug md:w-auto md:text-base">
              DigitalOcean acquires Katanemo Labs, Inc. to accelerate AI
              development
            </span>
            <span className="hidden shrink-0 items-center gap-1 text-base font-medium tracking-[-0.989px] font-mono leading-snug opacity-70 transition-opacity hover:opacity-100 md:inline-flex">
              Read the announcement
              <ArrowRightIcon aria-hidden className="h-3.5 w-3.5 text-white/70" />
            </span>
          </div>
        </Link>
        <ConditionalLayout>{children}</ConditionalLayout>
        <Analytics />
      </body>
    </html>
  );
}
