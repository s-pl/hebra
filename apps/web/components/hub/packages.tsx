import type { Package } from "@/lib/hub";

/** A package's address in the hub. */
export function packageHref(owner: string, p: Pick<Package, "kind" | "name">): string {
  return `/hub/${owner}/packages/${p.kind}/${p.name.split("/").map(encodeURIComponent).join("/")}`;
}

export function KindIcon({ kind }: { kind: string }) {
  return (
    <span className="grid size-9 shrink-0 place-items-center rounded-lg bg-wash text-ink-2" aria-hidden="true">
      {kind === "npm" ? (
        <svg
          width="18"
          height="18"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.6"
          aria-hidden="true"
        >
          <path d="M12 2.5 3.5 7v10L12 21.5 20.5 17V7L12 2.5Z" strokeLinejoin="round" />
          <path d="M3.5 7 12 11.5 20.5 7M12 11.5v10" strokeLinejoin="round" />
        </svg>
      ) : (
        <svg
          width="18"
          height="18"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.6"
          aria-hidden="true"
        >
          <rect x="3" y="6" width="18" height="12" rx="2" />
          <path d="M7 6v12M11 6v12M15 6v12" />
        </svg>
      )}
    </span>
  );
}
