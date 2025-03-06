import { useRef, useEffect, useCallback } from "react";

export function useResizableElement({
  minWidth = 280,
  maxWidth = 550,
  defaultWidth = 350,
}) {
  const elementRef = useRef(null);
  const resizerRef = useRef(null);
  const isResizingRef = useRef(false);

  const startResizing = useCallback((e) => {
    isResizingRef.current = true;
    document.body.style.userSelect = "none"; // Prevent text selection while resizing
  }, []);

  const stopResizing = useCallback(() => {
    if (!isResizingRef.current) return;
    isResizingRef.current = false;
    document.body.style.userSelect = "";
  }, []);

  const resize = useCallback(
    (e) => {
      if (!isResizingRef.current || !elementRef.current) return;

      const newWidth = window.innerWidth - e.clientX;

      if (newWidth >= minWidth && newWidth <= maxWidth) {
        elementRef.current.style.width = `${newWidth}px`;
      }
    },
    [minWidth, maxWidth]
  );

  useEffect(() => {
    const resizer = resizerRef.current;

    if (resizer) {
      resizer.addEventListener("mousedown", startResizing);
    }

    document.addEventListener("mousemove", resize);
    document.addEventListener("mouseup", stopResizing);

    return () => {
      if (resizer) {
        resizer.removeEventListener("mousedown", startResizing);
      }

      document.removeEventListener("mousemove", resize);
      document.removeEventListener("mouseup", stopResizing);
    };
  }, [startResizing, resize, stopResizing]);

  return { elementRef, resizerRef };
}
