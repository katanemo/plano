import type { Metadata } from "next";
import Script from "next/script";
import localFont from "next/font/local";
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

export const metadata: Metadata = {
  title: "Katanemo Labs",
  description:
    "Forward-deployed AI infrastructure engineers delivering industry-leading research and open-source technologies,",
  icons: {
    icon: "/KatanemoLogo.svg",
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
