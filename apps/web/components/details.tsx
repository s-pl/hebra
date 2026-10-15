/** Bento grid of the small, correct details in Hebra's core. All claims are implemented and tested. */

import type { ReactNode } from "react";
import { Spotlight } from "@/components/spotlight";

function Card({
  title,
  body,
  children,
  wide = false,
}: {
  title: string;
  body: string;
  children: ReactNode;
  wide?: boolean;
}) {
  return (
    <div
      className={`spotlight-card card-hover flex flex-col rounded-lg border border-line bg-surface p-5 ${wide ? "lg:col-span-2" : ""}`}
    >
      <div className="flex min-h-[92px] items-center justify-center rounded-lg border border-line border-dashed bg-surface-2/60 px-3 py-4 font-mono text-[12px]">
        {children}
      </div>
      <h3 className="mt-4 font-semibold text-[15px]">{title}</h3>
      <p className="mt-1 text-[14px] text-ink-2 leading-relaxed">{body}</p>
    </div>
  );
}

export function Details() {
  return (
    <Spotlight className="stagger grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
      <Card
        title="Stable error codes"
        body="Errors carry a fixed code like revision_not_found. Agents check the code instead of matching message text."
        wide
      >
        <code className="text-ink-2">
          {"{ "}
          <span className="text-ink-3">"error"</span>
          {": { "}
          <span className="text-ink-3">"code"</span>: <span className="text-accent">"revision_not_found"</span>
          {" } }"}
        </code>
      </Card>
      <Card title="Undo anything" body="Each command is an entry in the operation log, and any entry can be reverted.">
        <ul className="grid w-full gap-1 text-ink-2">
          <li className="flex justify-between">
            <span>11 undo</span>
            <span className="text-accent">↩ 10</span>
          </li>
          <li className="flex justify-between text-ink-3 line-through">
            <span>10 merge</span>
            <span>ana</span>
          </li>
          <li className="flex justify-between">
            <span>9 commit</span>
            <span>agent-2</span>
          </li>
        </ul>
      </Card>
      <Card
        title="Line endings, solved"
        body="CRLF becomes LF when a file is recorded, on every OS, with no settings to get wrong."
      >
        <span className="text-ink-2">
          a<span className="text-accent line-through">\r</span>\n → a\n
        </span>
      </Card>
      <Card
        title="Tamper-evident"
        body="Every object is named by its BLAKE3 hash and checked against it when it is read back."
      >
        <span className="flex items-center gap-2 text-ink-2">
          blake3:f2d1d18…
          <span className="text-[oklch(0.6_0.14_150)]">✓</span>
        </span>
      </Card>
      <Card
        title="Your .gitignore just works"
        body="hb init imports existing files and honors .gitignore and .hbignore."
      >
        <span className="grid gap-0.5 text-left text-ink-2">
          <span>src/main.rs</span>
          <span className="text-ink-3 line-through">target/debug/app</span>
          <span className="text-ink-3 line-through">node_modules/</span>
        </span>
      </Card>
      <Card
        title="No escaping the workspace"
        body="Paths are checked before anything is written, so a bad merge can't write outside the workspace."
      >
        <span className="text-ink-2">
          ../../etc/passwd → <span className="text-accent">invalid_path</span>
        </span>
      </Card>
      <Card
        title="Atomic by design"
        body="Objects go to a temp file, get synced, then renamed. History changes run in SQLite transactions."
      >
        <span className="text-ink-2">tmp → fsync → rename</span>
      </Card>
    </Spotlight>
  );
}
