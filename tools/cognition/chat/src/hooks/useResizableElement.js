import { useRef, useEffect, useCallback } from "react";

export function useResizableElement({
  minWidth = 280,
  maxWidth = 550,
  defaultWidth = 350,
}) {
  const elementRef = useRef(null);
  const resizerRef = useRef(null);
  const isResizingRef = useRef(false);
  // Track width with refs instead of state to avoid re-renders
  const currentWidthRef = useRef(defaultWidth);
  const lastWidthRef = useRef(defaultWidth);

  const startResizing = useCallback((e) => {
    isResizingRef.current = true;
    document.body.style.userSelect = "none"; // Prevent text selection while resizing
    console.log("[useResizableElement] Started resizing");
  }, []);

  const stopResizing = useCallback(() => {
    if (!isResizingRef.current) return;
    isResizingRef.current = false;
    document.body.style.userSelect = "";

    // Store the last width after resizing stops
    if (elementRef.current) {
      const width = parseInt(getComputedStyle(elementRef.current).width);
      lastWidthRef.current = width;
      currentWidthRef.current = width;
      console.log(
        `[useResizableElement] Stopped resizing. Final width: ${width}px`
      );
    }
  }, []);

  const resize = useCallback(
    (e) => {
      if (!isResizingRef.current || !elementRef.current) return;

      const newWidth = window.innerWidth - e.clientX;

      if (newWidth >= minWidth && newWidth <= maxWidth) {
        elementRef.current.style.width = `${newWidth}px`;
        // Update ref but not state to avoid re-renders
        currentWidthRef.current = newWidth;
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

    // Set initial width if not already set
    if (elementRef.current && !elementRef.current.style.width) {
      elementRef.current.style.width = `${defaultWidth}px`;
      console.log(`[useResizableElement] Set initial width: ${defaultWidth}px`);
    }

    return () => {
      if (resizer) {
        resizer.removeEventListener("mousedown", startResizing);
      }

      document.removeEventListener("mousemove", resize);
      document.removeEventListener("mouseup", stopResizing);
    };
  }, [startResizing, resize, stopResizing, defaultWidth]);

  // Function to save current width
  const saveCurrentWidth = useCallback(() => {
    if (elementRef.current) {
      const width = parseInt(getComputedStyle(elementRef.current).width);
      lastWidthRef.current = width;
      return width;
    }
    return lastWidthRef.current;
  }, []);

  // Function to restore saved width
  const restoreWidth = useCallback(() => {
    if (elementRef.current && lastWidthRef.current) {
      elementRef.current.style.width = `${lastWidthRef.current}px`;
    }
  }, []);

  return {
    elementRef,
    resizerRef,
    currentWidthRef,
    lastWidthRef,
    saveCurrentWidth,
    restoreWidth,
  };
}
