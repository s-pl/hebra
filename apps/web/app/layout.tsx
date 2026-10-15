import type { Metadata, Viewport } from "next";
import { Geist, Geist_Mono, Instrument_Serif } from "next/font/google";
import type { ReactNode } from "react";
import { SITE } from "@/lib/site";
import "./globals.css";

const sans = Geist({ subsets: ["latin"], variable: "--font-geist-sans" });
const mono = Geist_Mono({ subsets: ["latin"], variable: "--font-geist-mono" });
const serif = Instrument_Serif({
  subsets: ["latin"],
  weight: "400",
  style: "italic",
  variable: "--font-instrument-serif",
});

export const metadata: Metadata = {
  metadataBase: new URL(SITE.url),
  title: "Hebra: version control for many agents at once",
  description: SITE.description,
  openGraph: {
    title: "Hebra: version control for many agents at once",
    description: SITE.description,
    url: SITE.url,
    siteName: "Hebra",
    type: "website",
  },
  twitter: { card: "summary_large_image" },
};

export const viewport: Viewport = {
  themeColor: [
    { media: "(prefers-color-scheme: light)", color: "#faf8f3" },
    { media: "(prefers-color-scheme: dark)", color: "#1b1917" },
  ],
};

// Applies the saved theme before first paint, so there is no flash.
const themeScript = `try{if(localStorage.getItem("hebra-theme")==="dark"){document.documentElement.dataset.theme="dark"}}catch(e){}`;

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" className={`${sans.variable} ${mono.variable} ${serif.variable}`} suppressHydrationWarning>
      <head>
        {/* biome-ignore lint/security/noDangerouslySetInnerHtml: static inline script, no user input */}
        <script dangerouslySetInnerHTML={{ __html: themeScript }} />
      </head>
      <body className="min-h-dvh">
        <a
          href="#main"
          className="sr-only focus:not-sr-only focus:fixed focus:top-3 focus:left-3 focus:z-50 focus:rounded-md focus:bg-surface focus:px-3 focus:py-2"
        >
          Skip to content
        </a>
        {children}
        <div aria-hidden="true" className="grain" />
      </body>
    </html>
  );
}
