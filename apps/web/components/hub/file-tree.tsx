import Link from "next/link";

type Dir = { dirs: Map<string, Dir>; files: string[] };

function build(paths: string[]): Dir {
  const root: Dir = { dirs: new Map(), files: [] };
  for (const p of paths) {
    const parts = p.split("/");
    let node = root;
    for (const part of parts.slice(0, -1)) {
      let next = node.dirs.get(part);
      if (!next) {
        next = { dirs: new Map(), files: [] };
        node.dirs.set(part, next);
      }
      node = next;
    }
    node.files.push(parts[parts.length - 1] ?? p);
  }
  return root;
}

function href(base: string, path: string) {
  return path ? `${base}/${path.split("/").map(encodeURIComponent).join("/")}` : base;
}

function Folder({
  name,
  dir,
  prefix,
  base,
  current,
  depth,
}: {
  name: string;
  dir: Dir;
  prefix: string;
  base: string;
  current: string;
  depth: number;
}) {
  const path = prefix ? `${prefix}/${name}` : name;
  const open = current === path || current.startsWith(`${path}/`);
  return (
    <li>
      <details open={open} className="group/dir">
        <summary
          className={`flex h-7 cursor-pointer list-none items-center gap-1.5 rounded-md pr-2 text-[13px] hover:bg-[color-mix(in_oklch,var(--text)_5%,transparent)] ${
            current === path ? "text-ink" : "text-ink-2"
          }`}
          style={{ paddingLeft: 8 + depth * 12 }}
        >
          <svg
            viewBox="0 0 12 12"
            className="size-2.5 shrink-0 text-ink-3 transition-transform duration-150 group-[:not([open])]/dir:-rotate-90"
            fill="currentColor"
            aria-hidden="true"
          >
            <path d="M2 4h8L6 9z" />
          </svg>
          <svg
            viewBox="0 0 16 16"
            className="size-3.5 shrink-0 text-[var(--thread-3)]"
            fill="currentColor"
            aria-hidden="true"
          >
            <path d="M1.5 3.5A1.5 1.5 0 0 1 3 2h3.3l1.5 1.5H13A1.5 1.5 0 0 1 14.5 5v7A1.5 1.5 0 0 1 13 13.5H3A1.5 1.5 0 0 1 1.5 12z" />
          </svg>
          <span className="truncate">{name}</span>
        </summary>
        <Entries dir={dir} prefix={path} base={base} current={current} depth={depth + 1} />
      </details>
    </li>
  );
}

function Entries({
  dir,
  prefix,
  base,
  current,
  depth,
}: {
  dir: Dir;
  prefix: string;
  base: string;
  current: string;
  depth: number;
}) {
  const folders = [...dir.dirs.entries()].sort(([a], [b]) => a.localeCompare(b));
  const files = [...dir.files].sort((a, b) => a.localeCompare(b));
  return (
    <ul>
      {folders.map(([name, sub]) => (
        <Folder key={name} name={name} dir={sub} prefix={prefix} base={base} current={current} depth={depth} />
      ))}
      {files.map((name) => {
        const path = prefix ? `${prefix}/${name}` : name;
        const active = current === path;
        return (
          <li key={name}>
            <Link
              href={href(base, path)}
              prefetch={false}
              aria-current={active ? "page" : undefined}
              className={`flex h-7 items-center gap-1.5 rounded-md pr-2 text-[13px] ${
                active
                  ? "bg-[color-mix(in_oklch,var(--accent)_12%,transparent)] font-medium text-ink"
                  : "text-ink-2 hover:bg-[color-mix(in_oklch,var(--text)_5%,transparent)] hover:text-ink"
              }`}
              style={{ paddingLeft: 8 + depth * 12 + 16 }}
            >
              <svg
                viewBox="0 0 16 16"
                className="size-3.5 shrink-0 text-ink-3"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.3"
                aria-hidden="true"
              >
                <path d="M4 1.5h5L12.5 5v9.5h-8.5z M9 1.5V5h3.5" strokeLinejoin="round" />
              </svg>
              <span className="truncate">{name}</span>
            </Link>
          </li>
        );
      })}
    </ul>
  );
}

/** Collapsible explorer of every file in a tree; folders on the current path start open. */
export function FileTree({ files, base, current }: { files: string[]; base: string; current: string }) {
  return <Entries dir={build(files)} prefix="" base={base} current={current} depth={0} />;
}

export { href as treeHref };
