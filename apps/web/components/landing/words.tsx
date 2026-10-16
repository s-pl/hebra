/**
 * A line of text whose words arrive one after another (CSS only, so it
 * plays before any JavaScript loads). Words stay inline blocks with real
 * spaces between them, so wrapping and screen readers are unaffected.
 */
export function Words({ text, start = 0, step = 60 }: { text: string; start?: number; step?: number }) {
  const words = text.split(" ");
  return (
    <>
      {words.map((w, i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: a fixed sentence; words can repeat
        <span key={i}>
          <span className="word" style={{ animationDelay: `${start + i * step}ms` }}>
            {w}
          </span>
          {i < words.length - 1 ? " " : null}
        </span>
      ))}
    </>
  );
}
