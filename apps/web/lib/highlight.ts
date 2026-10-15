import { bundledLanguages, bundledLanguagesAlias, codeToHtml } from "shiki";

/** Files bigger than this are shown without highlighting, to stay fast. */
const MAX_BYTES = 300_000;
const MAX_LINES = 8_000;

const BY_NAME: Record<string, string> = {
  Dockerfile: "docker",
  Makefile: "make",
  "Cargo.lock": "toml",
  ".gitignore": "shellscript",
  ".hbignore": "shellscript",
};

export function languageFor(path: string): string {
  const name = path.split("/").pop() ?? path;
  const byName = BY_NAME[name];
  if (byName) return byName;
  const ext = name.includes(".") ? (name.split(".").pop() ?? "").toLowerCase() : "";
  if (ext && (ext in bundledLanguages || ext in bundledLanguagesAlias)) return ext;
  return "text";
}

/**
 * Server-side syntax highlighting with VS Code grammars. Colours are emitted
 * as CSS variables for both themes, so switching light/dark costs nothing.
 */
export async function highlight(code: string, path: string): Promise<string | null> {
  if (code.length > MAX_BYTES || code.split("\n").length > MAX_LINES) return null;
  try {
    return await codeToHtml(code, {
      lang: languageFor(path),
      themes: { light: "github-light", dark: "github-dark-dimmed" },
      defaultColor: false,
    });
  } catch {
    return null;
  }
}
