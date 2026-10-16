import type { Metadata, Viewport } from "next";
import { Geist, Geist_Mono } from "next/font/google";
import { headers } from "next/headers";
import { type ReactNode, Suspense } from "react";
import { NavProgress } from "@/components/hub/nav-progress";
import { SITE } from "@/lib/site";
import "./globals.css";

const sans = Geist({ subsets: ["latin"], variable: "--font-geist-sans" });
const mono = Geist_Mono({ subsets: ["latin"], variable: "--font-geist-mono" });

export const metadata: Metadata = {
  metadataBase: new URL(SITE.url),
  title: { default: "Hebra: version control for people and agents", template: "%s · Hebra" },
  description: SITE.description,
  openGraph: {
    title: "Hebra: version control for people and agents",
    description: SITE.description,
    url: SITE.url,
    siteName: "Hebra",
    type: "website",
  },
  twitter: { card: "summary_large_image" },
};

export const viewport: Viewport = {
  themeColor: [
    { media: "(prefers-color-scheme: dark)", color: "#0e0f12" },
    { media: "(prefers-color-scheme: light)", color: "#fcfcfd" },
  ],
};

// Applies a saved light theme before first paint, so there is no flash.
const themeScript = `try{if(localStorage.getItem("hebra-theme")==="light"){document.documentElement.dataset.theme="light"}}catch(e){}`;

export default async function RootLayout({ children }: { children: ReactNode }) {
  // Set by proxy.ts; the CSP only lets scripts with this nonce run.
  const nonce = (await headers()).get("x-nonce") ?? undefined;
  return (
    <html lang="en" className={`${sans.variable} ${mono.variable}`} suppressHydrationWarning>
      <head>
        {/* biome-ignore lint/security/noDangerouslySetInnerHtml: static inline script, no user input */}
        <script nonce={nonce} dangerouslySetInnerHTML={{ __html: themeScript }} />
      </head>
      <body className="min-h-dvh">
        <a
          href="#main"
          className="sr-only focus:not-sr-only focus:fixed focus:top-3 focus:left-3 focus:z-50 focus:rounded-md focus:bg-surface-2 focus:px-3 focus:py-2"
        >
          Skip to content
        </a>
        {children}
        {/* In the root layout so it lives across navigations. */}
        <Suspense>
          <NavProgress />
        </Suspense>
      </body>
    </html>
  );
}
