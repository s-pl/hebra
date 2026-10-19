import type { Metadata, Viewport } from "next";
import { Hanken_Grotesk, JetBrains_Mono } from "next/font/google";
import { headers } from "next/headers";
import { type ReactNode, Suspense } from "react";
import { NavProgress } from "@/components/hub/nav-progress";
import { SITE } from "@/lib/site";
import "./globals.css";

const sans = Hanken_Grotesk({ subsets: ["latin"], variable: "--font-hanken" });
const mono = JetBrains_Mono({ subsets: ["latin"], variable: "--font-jetbrains" });

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
    { media: "(prefers-color-scheme: dark)", color: "#111113" },
    { media: "(prefers-color-scheme: light)", color: "#e6e5df" },
  ],
};

// Applies a saved dark theme before first paint, so there is no flash.
const themeScript = `try{if(localStorage.getItem("hebra-theme")==="dark"){document.documentElement.dataset.theme="dark"}}catch(e){}`;

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
