import type { Metadata } from "next";
import Script from "next/script";
import localFont from "next/font/local";
import { siteConfig } from "../lib/metadata";
import "@katanemo/shared-styles/globals.css";
import "./globals.css";

const ibmPlexSans = localFont({
  src: [
    {
      path: "../../../www/public/fonts/IBMPlexSans-VariableFont_wdth,wght.ttf",
      weight: "100 700",
      style: "normal",
    },
    {
      path: "../../../www/public/fonts/IBMPlexSans-Italic-VariableFont_wdth,wght.ttf",
      weight: "100 700",
      style: "italic",
    },
  ],
  display: "swap",
  variable: "--font-ibm-plex-sans",
});

const baseUrl = new URL(siteConfig.url);

export const metadata: Metadata = {
  title: `${siteConfig.name} - ${siteConfig.tagline}`,
  description: siteConfig.description,
  keywords: siteConfig.keywords,
  metadataBase: baseUrl,
  authors: siteConfig.authors,
  creator: siteConfig.creator,
  icons: {
    icon: "/KatanemoLogo.svg",
  },
  openGraph: {
    type: "website",
    locale: "en_US",
    url: siteConfig.url,
    title: `${siteConfig.name} - ${siteConfig.tagline}`,
    description: siteConfig.description,
    siteName: siteConfig.name,
    images: [
      {
        url: siteConfig.ogImage,
        width: 1200,
        height: 630,
        alt: `${siteConfig.name} - ${siteConfig.tagline}`,
      },
    ],
  },
  twitter: {
    card: "summary_large_image",
    title: `${siteConfig.name} - ${siteConfig.tagline}`,
    description: siteConfig.description,
    images: [siteConfig.ogImage],
    creator: "@katanemo",
  },
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="en">
      <body className={`${ibmPlexSans.variable} antialiased text-white`}>
        {/* Google tag (gtag.js) */}
        <Script
          src="https://www.googletagmanager.com/gtag/js?id=G-RLD5BDNW5N"
          strategy="afterInteractive"
        />
        <Script strategy="afterInteractive">
          {`
            window.dataLayer = window.dataLayer || [];
            function gtag(){dataLayer.push(arguments);}
            gtag('js', new Date());
            gtag('config', 'G-RLD5BDNW5N');
          `}
        </Script>
        <div className="min-h-screen">{children}</div>
      </body>
    </html>
  );
}
