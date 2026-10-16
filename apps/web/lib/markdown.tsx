import type { ReactNode } from "react";

/**
 * A small Markdown renderer for the site's own documents (the billing
 * policy): headings, paragraphs, lists, tables, rules, emphasis, code and
 * links. It builds React elements, never HTML strings, so text is always
 * escaped. Not meant for user content.
 */

function inline(text: string, key = "i"): ReactNode[] {
  const out: ReactNode[] = [];
  const pattern = /(\*\*[^*]+\*\*|\*[^*]+\*|`[^`]+`|\[[^\]]+\]\([^)]+\))/g;
  let last = 0;
  let n = 0;
  for (const m of text.matchAll(pattern)) {
    const at = m.index ?? 0;
    if (at > last) out.push(text.slice(last, at));
    const token = m[0];
    const k = `${key}-${n++}`;
    if (token.startsWith("**")) out.push(<strong key={k}>{inline(token.slice(2, -2), k)}</strong>);
    else if (token.startsWith("`"))
      out.push(
        <code key={k} className="rounded bg-surface-2 px-1 py-px font-mono text-[0.9em]">
          {token.slice(1, -1)}
        </code>,
      );
    else if (token.startsWith("[")) {
      const [, label = "", href = ""] = /^\[([^\]]+)\]\(([^)]+)\)$/.exec(token) ?? [];
      // Links to repository files go to GitHub; only http(s) and site paths are kept.
      const url = href.startsWith("../")
        ? `https://github.com/s-pl/hebra/blob/main/${href.replace(/^(\.\.\/)+/, "")}`
        : href;
      out.push(
        /^(https?:\/\/|\/|#)/.test(url) ? (
          <a key={k} href={url} className="underline underline-offset-4 hover:text-ink">
            {inline(label, k)}
          </a>
        ) : (
          label
        ),
      );
    } else out.push(<em key={k}>{inline(token.slice(1, -1), k)}</em>);
    last = at + token.length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

const cells = (row: string) =>
  row
    .trim()
    .replace(/^\||\|$/g, "")
    .split("|")
    .map((c) => c.trim());

export function slug(text: string) {
  return text
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");
}

export function headings(source: string): { id: string; text: string }[] {
  return source
    .split("\n")
    .filter((l) => l.startsWith("## "))
    .map((l) => ({ id: slug(l.slice(3)), text: l.slice(3).trim() }));
}

export function Markdown({ source, skipTitle = false }: { source: string; skipTitle?: boolean }) {
  const lines = source.replace(/\r\n/g, "\n").split("\n");
  const blocks: ReactNode[] = [];
  let i = 0;
  let b = 0;
  while (i < lines.length) {
    const line = lines[i] ?? "";
    const key = `b${b++}`;
    if (!line.trim()) {
      i++;
    } else if (line.startsWith("# ")) {
      if (!skipTitle)
        blocks.push(
          <h1 key={key} className="font-semibold text-[34px] tracking-[-0.03em]">
            {inline(line.slice(2))}
          </h1>,
        );
      i++;
    } else if (line.startsWith("## ")) {
      const text = line.slice(3).trim();
      blocks.push(
        <h2 key={key} id={slug(text)} className="mt-12 scroll-mt-24 font-semibold text-[22px] tracking-[-0.02em]">
          {inline(text)}
        </h2>,
      );
      i++;
    } else if (/^-{3,}$/.test(line.trim())) {
      blocks.push(<hr key={key} className="my-10 border-line" />);
      i++;
    } else if (line.startsWith("|")) {
      const rows: string[] = [];
      while (i < lines.length && (lines[i] ?? "").startsWith("|")) rows.push(lines[i++] ?? "");
      const [head = "", , ...body] = rows;
      blocks.push(
        <div key={key} className="mt-5 overflow-x-auto rounded-xl border border-line">
          <table className="w-full min-w-[640px] text-left text-[13.5px]">
            <thead>
              <tr className="border-line border-b bg-surface-2">
                {cells(head).map((c, j) => (
                  // biome-ignore lint/suspicious/noArrayIndexKey: static document
                  <th key={j} scope="col" className="px-3 py-2 font-medium">
                    {inline(c)}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {body.map((r, j) => (
                // biome-ignore lint/suspicious/noArrayIndexKey: static document
                <tr key={j} className="border-line border-b last:border-b-0">
                  {cells(r).map((c, k) => (
                    // biome-ignore lint/suspicious/noArrayIndexKey: static document
                    <td key={k} className={`px-3 py-2 ${k === 0 ? "text-ink-2" : ""}`}>
                      {inline(c)}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>,
      );
    } else if (/^(\d+\.|-) /.test(line)) {
      const ordered = /^\d+\./.test(line);
      const items: string[] = [];
      while (i < lines.length && (lines[i] ?? "").trim()) {
        const l = lines[i++] ?? "";
        if (/^(\d+\.|-) /.test(l)) items.push(l.replace(/^(\d+\.|-) /, ""));
        else items[items.length - 1] += ` ${l.trim()}`;
      }
      const List = ordered ? "ol" : "ul";
      blocks.push(
        <List
          key={key}
          className={`mt-4 grid gap-2 pl-5 text-[15.5px] text-ink-2 leading-relaxed ${ordered ? "list-decimal" : "list-disc"}`}
        >
          {items.map((it, j) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: static document
            <li key={j}>{inline(it, `${key}-${j}`)}</li>
          ))}
        </List>,
      );
    } else {
      // The first line always belongs to the paragraph, so the loop advances.
      const para: string[] = [(lines[i++] ?? "").replace(/^#+ /, "").trim()];
      while (i < lines.length && (lines[i] ?? "").trim() && !/^(#|\||- |\d+\. |-{3,})/.test(lines[i] ?? ""))
        para.push((lines[i++] ?? "").trim());
      blocks.push(
        <p key={key} className="mt-4 text-[15.5px] text-ink-2 leading-relaxed">
          {inline(para.join(" "), key)}
        </p>,
      );
    }
  }
  return <>{blocks}</>;
}
