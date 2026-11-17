import { useEffect, useState } from "react";

/** Pages as spreads: the first page is a recto alone, then verso + recto pairs. */
export function Preview({ pages }: { pages: string[] }) {
  const [urls, setUrls] = useState<string[]>([]);
  useEffect(() => {
    // Blob URLs in <img>: the SVG can't run scripts or touch the app's DOM.
    const next = pages.map((svg) => URL.createObjectURL(new Blob([svg], { type: "image/svg+xml" })));
    setUrls(next);
    return () => next.forEach((u) => URL.revokeObjectURL(u));
  }, [pages]);

  const spreads: (string | null)[][] = [];
  if (urls.length > 0) spreads.push([null, urls[0]]);
  for (let i = 1; i < urls.length; i += 2) spreads.push([urls[i], urls[i + 1] ?? null]);

  return (
    <div className="preview">
      {spreads.map((spread, i) => (
        <div className="spread" key={i}>
          {spread.map((url, j) =>
            url ? <img key={j} src={url} alt={`Page ${i * 2 + j}`} /> : <div key={j} className="page-slot" />,
          )}
        </div>
      ))}
    </div>
  );
}
