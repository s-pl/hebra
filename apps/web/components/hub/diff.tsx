import type { FileDiff } from "@/lib/hub";

type Line = { kind: "add" | "del" | "ctx" | "hunk"; text: string; old?: number; new?: number };

/** Parses a unified patch into lines with old and new line numbers. */
function parse(patch: string): Line[] {
  const out: Line[] = [];
  let oldNo = 0;
  let newNo = 0;
  for (const raw of patch.split("\n")) {
    if (raw.startsWith("--- ") || raw.startsWith("+++ ")) continue;
    const hunk = raw.match(/^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@(.*)$/);
    if (hunk) {
      oldNo = Number(hunk[1]);
      newNo = Number(hunk[2]);
      out.push({ kind: "hunk", text: raw });
      continue;
    }
    if (raw.startsWith("\\")) continue;
    if (raw.startsWith("+")) out.push({ kind: "add", text: raw.slice(1), new: newNo++ });
    else if (raw.startsWith("-")) out.push({ kind: "del", text: raw.slice(1), old: oldNo++ });
    else if (raw.length > 0 || out.length > 0)
      out.push({ kind: "ctx", text: raw.slice(1), old: oldNo++, new: newNo++ });
  }
  while (out.length > 0 && out[out.length - 1]?.kind === "ctx" && out[out.length - 1]?.text === "") out.pop();
  return out;
}

const STATUS = {
  added: { letter: "A", tone: "text-ok" },
  modified: { letter: "M", tone: "text-[var(--thread-3)]" },
  deleted: { letter: "D", tone: "text-err" },
};

export function fileAnchor(path: string): string {
  return `file-${path.replace(/[^a-zA-Z0-9]+/g, "-")}`;
}

/** Stat bar: five squares split between additions and deletions. */
export function DiffStat({ additions, deletions }: { additions: number; deletions: number }) {
  const total = additions + deletions;
  const greens = total === 0 ? 0 : Math.round((additions / total) * 5);
  return (
    <span className="inline-flex items-center gap-1.5 font-mono text-[12px]">
      <span className="text-ok">+{additions}</span>
      <span className="text-err">−{deletions}</span>
      <span className="flex gap-px" aria-hidden="true">
        {Array.from({ length: 5 }, (_, i) => (
          <span
            // biome-ignore lint/suspicious/noArrayIndexKey: fixed five squares
            key={i}
            className={`size-[7px] rounded-[1.5px] ${total === 0 ? "bg-line" : i < greens ? "bg-ok" : "bg-err"}`}
          />
        ))}
      </span>
    </span>
  );
}

export function DiffFile({ file }: { file: FileDiff }) {
  const lines = parse(file.patch ?? "");
  const status = STATUS[file.status];
  return (
    <details
      open
      id={fileAnchor(file.path)}
      className="diff-file group scroll-mt-16 overflow-hidden rounded-lg border border-line bg-surface"
    >
      <summary className="sticky top-12 z-10 flex cursor-pointer list-none items-center gap-3 border-line border-b bg-surface-2 px-4 py-2 group-[:not([open])]:border-b-0">
        <svg
          viewBox="0 0 12 12"
          className="size-3 shrink-0 text-ink-3 transition-transform duration-150 group-[:not([open])]:-rotate-90"
          fill="currentColor"
          aria-hidden="true"
        >
          <path d="M2 4h8L6 9z" />
        </svg>
        <span className={`font-mono font-semibold text-[12px] ${status.tone}`}>{status.letter}</span>
        <span className="min-w-0 flex-1 truncate font-mono text-[13px]">{file.path}</span>
        <DiffStat additions={file.additions} deletions={file.deletions} />
      </summary>
      {file.binary ? (
        <p className="px-4 py-4 text-ink-3 text-sm">Binary file not shown.</p>
      ) : (
        <div className="overflow-x-auto">
          <table className="w-full border-collapse font-mono text-[12.5px] leading-[1.7]">
            <tbody>
              {lines.map((l, i) =>
                l.kind === "hunk" ? (
                  // biome-ignore lint/suspicious/noArrayIndexKey: diff lines are static
                  <tr key={i} className="bg-[color-mix(in_oklch,var(--info)_7%,transparent)]">
                    <td colSpan={3} className="px-4 py-1 text-[11.5px] text-ink-3">
                      {l.text}
                    </td>
                  </tr>
                ) : (
                  <tr
                    // biome-ignore lint/suspicious/noArrayIndexKey: diff lines are static
                    key={i}
                    className={
                      l.kind === "add"
                        ? "bg-[color-mix(in_oklch,var(--ok)_11%,transparent)]"
                        : l.kind === "del"
                          ? "bg-[color-mix(in_oklch,var(--err)_10%,transparent)]"
                          : ""
                    }
                  >
                    <td className="w-12 select-none border-line border-r px-2 text-right align-top text-ink-3/70">
                      {l.old ?? ""}
                    </td>
                    <td className="w-12 select-none border-line border-r px-2 text-right align-top text-ink-3/70">
                      {l.new ?? ""}
                    </td>
                    <td className="whitespace-pre px-4">
                      <span
                        className={`mr-3 inline-block w-2 select-none ${
                          l.kind === "add" ? "text-ok" : l.kind === "del" ? "text-err" : "text-transparent"
                        }`}
                      >
                        {l.kind === "add" ? "+" : l.kind === "del" ? "−" : " "}
                      </span>
                      {l.text || " "}
                    </td>
                  </tr>
                ),
              )}
            </tbody>
          </table>
        </div>
      )}
    </details>
  );
}
