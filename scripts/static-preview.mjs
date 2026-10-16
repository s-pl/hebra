// Saves a running Hebra web (built with HEBRA_BASE_PATH) as static files, for
// a read-only preview on GitHub Pages: every page reachable from the start
// page, plus the scripts, styles, fonts and images they load.
//
//   HEBRA_BASE_PATH=/hebra pnpm build && HEBRA_BASE_PATH=/hebra pnpm start -p 3100
//   node scripts/static-preview.mjs http://127.0.0.1:3100 /hebra out/
//
// Forms and sign-in need the server, so they do nothing in the preview.
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

const [origin, base, outDir, maxPages = "400"] = process.argv.slice(2);
if (!origin || !base || !outDir) {
  console.error("usage: static-preview.mjs <origin> <base path> <out dir> [max pages]");
  process.exit(2);
}

// Pages that need a signed-in person or the server to act; not worth saving.
const SKIP = [/^\/auth\//, /^\/login\/(device|invite)/, /^\/settings/, /^\/hub\/new/, /^\/hub\/api\//, /^\/hub\/login/];

const pages = new Set();
const assets = new Set();
const queue = [base];
mkdirSync(outDir, { recursive: true });

function local(url) {
  try {
    const u = new URL(url, origin);
    if (u.origin !== new URL(origin).origin || !u.pathname.startsWith(base)) return null;
    return u;
  } catch {
    return null;
  }
}

function fileFor(pathname) {
  const rel = pathname.slice(base.length).replace(/^\/+/, "");
  // `page/index.html`, so a page and a folder below it never collide.
  return rel ? `${rel.replace(/\/$/, "")}/index.html` : "index.html";
}

/**
 * A thin bar on every page saying this is a snapshot. It is added to the
 * stylesheets (body::after), not the HTML, so React hydration still matches.
 */
const BANNER_CSS = `
body::after{content:"Static preview of Hebra with demo data. Sign-in and actions need a running server.";position:fixed;inset:auto 0 0 0;z-index:100;padding:6px 12px;font:12px/1.4 system-ui,sans-serif;text-align:center;background:#0b0c0e;color:#aeb4bd;border-top:1px solid #2a2d33;pointer-events:none}`;

function save(rel, body) {
  const path = join(outDir, decodeURIComponent(rel));
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, body);
}

while (queue.length > 0 && pages.size < Number(maxPages)) {
  const path = queue.shift();
  if (pages.has(path)) continue;
  pages.add(path);
  const res = await fetch(origin + path, { redirect: "manual" });
  const moved = res.status >= 300 && res.status < 400 ? local(res.headers.get("location") ?? "") : null;
  if (moved && !pages.has(moved.pathname)) queue.push(moved.pathname);
  if (res.status !== 200 || !(res.headers.get("content-type") ?? "").includes("text/html")) continue;
  const html = await res.text();
  save(fileFor(path), html);

  for (const m of html.matchAll(/(?:href|src)="([^"#]+)/g)) {
    const u = local(m[1].replaceAll("&amp;", "&"));
    if (!u) continue;
    if (u.pathname.includes("/_next/") || /\.(svg|png|ico|css|js|woff2?)$/.test(u.pathname)) {
      assets.add(u.pathname);
    } else {
      const route = u.pathname.slice(base.length) || "/";
      if (!SKIP.some((r) => r.test(route)) && !pages.has(u.pathname)) queue.push(u.pathname);
    }
  }
  // Scripts named in the inline flight data (lazy chunks).
  for (const m of html.matchAll(/(\/[^"\\\s]*?\/_next\/static\/[^"\\\s]+?\.(?:js|css))/g)) {
    const u = local(m[1]);
    if (u) assets.add(u.pathname);
  }
}

// Assets, and what stylesheets point to (fonts).
const done = new Set();
const pending = [...assets];
while (pending.length > 0) {
  const path = pending.shift();
  if (done.has(path)) continue;
  done.add(path);
  const res = await fetch(origin + path);
  if (!res.ok) continue;
  let buf = Buffer.from(await res.arrayBuffer());
  if (path.endsWith(".css")) buf = Buffer.concat([buf, Buffer.from(BANNER_CSS)]);
  save(path.slice(base.length).replace(/^\/+/, ""), buf);
  if (path.endsWith(".css")) {
    for (const m of buf.toString("utf8").matchAll(/url\(([^)]+)\)/g)) {
      const u = local(new URL(m[1].replace(/["']/g, ""), origin + path).href);
      if (u && !done.has(u.pathname)) pending.push(u.pathname);
    }
  }
}

// GitHub Pages runs Jekyll unless told not to, and Jekyll drops `_next`.
writeFileSync(join(outDir, ".nojekyll"), "");
console.log(JSON.stringify({ pages: [...pages].length, assets: done.size }));
