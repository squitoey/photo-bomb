import { useState } from "react";
import { fullUrl, Photo, thumbUrl } from "./api";

const VIDEO = ["mp4", "mov", "m4v"];
const ext = (p: Photo) => (p.path.split(".").pop() ?? "").toLowerCase();

/** False for videos and for files that could not be read as images. */
export const canPreview = (p: Photo) => p.decoded !== 2 && !VIDEO.includes(ext(p));

interface Props {
  photo: Photo;
  /** Full-size image instead of the small thumbnail. */
  full?: boolean;
  lazy?: boolean;
  /** Shown when there is no picture. Defaults to the file extension. */
  placeholder?: string;
}

/**
 * A photo's picture. Thumbnails are made on demand, so this works for photos the scan
 * has not analysed yet; if the picture cannot be loaded, a placeholder is shown instead.
 */
export function Thumb({ photo, full, lazy, placeholder }: Props) {
  const src = full ? fullUrl(photo) : thumbUrl(photo);
  const [failed, setFailed] = useState<string | null>(null);
  if (!canPreview(photo) || failed === src) {
    return <div className="noimg">{placeholder ?? ext(photo).toUpperCase()}</div>;
  }
  return <img src={src} loading={lazy ? "lazy" : undefined} alt="" onError={() => setFailed(src)} />;
}
