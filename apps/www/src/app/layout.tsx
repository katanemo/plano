import type { Metadata } from "next";
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
        <ConditionalLayout>{children}</ConditionalLayout>
        <Analytics />
      </body>
    </html>
  );
}
