import type { NextConfig } from "next";

/** Sent with every response, static files included (the CSP is in proxy.ts). */
const securityHeaders = [
  { key: "Strict-Transport-Security", value: "max-age=63072000; includeSubDomains; preload" },
  { key: "X-Content-Type-Options", value: "nosniff" },
  { key: "X-Frame-Options", value: "DENY" },
  { key: "Referrer-Policy", value: "strict-origin-when-cross-origin" },
  { key: "Cross-Origin-Opener-Policy", value: "same-origin" },
  { key: "Cross-Origin-Resource-Policy", value: "same-origin" },
  {
    key: "Permissions-Policy",
    value: "camera=(), microphone=(), geolocation=(), payment=(), usb=(), interest-cohort=()",
  },
];

const config: NextConfig = {
  reactStrictMode: true,
  poweredByHeader: false,
  // A self-contained server for the container image (deploy/docker/web.Dockerfile).
  output: "standalone",
  // Served under a path prefix (the static preview on GitHub Pages: /hebra).
  basePath: process.env.HEBRA_BASE_PATH || undefined,
  // Plain asset URLs (stickers) need the prefix too; Next adds it to links only.
  env: { NEXT_PUBLIC_BASE_PATH: process.env.HEBRA_BASE_PATH ?? "" },
  // `next dev` opened from another device (a phone on the LAN): HEBRA_DEV_ORIGINS=192.168.1.16
  allowedDevOrigins: (process.env.HEBRA_DEV_ORIGINS ?? "").split(",").filter(Boolean),
  async headers() {
    return [{ source: "/:path*", headers: securityHeaders }];
  },
};

export default config;
