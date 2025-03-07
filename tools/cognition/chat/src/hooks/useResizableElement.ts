import { useRef, useEffect, useCallback } from "react";

interface ResizableElementOptions {
  minWidth?: number;
  maxWidth?: number;
  defaultWidth?: number;
}

interface ResizableElementResult {
  elementRef: React.RefObject<HTMLDivElement | null>;
  resizerRef: React.RefObject<HTMLDivElement | null>;
  currentWidthRef: React.MutableRefObject<number>;
  lastWidthRef: React.MutableRefObject<number>;
  saveCurrentWidth: () => number;
  restoreWidth: () => void;
}

export function useResizableElement({
  minWidth = 280,
  maxWidth = 550,
  defaultWidth = 350,
}: ResizableElementOptions = {}): ResizableElementResult {
  const elementRef = useRef<HTMLDivElement>(null);
  const resizerRef = useRef<HTMLDivElement>(null);
  const isResizingRef = useRef<boolean>(false);
  // Track width with refs instead of state to avoid re-renders
  const currentWidthRef = useRef<number>(defaultWidth);
  const lastWidthRef = useRef<number>(defaultWidth);

  const startResizing = useCallback((e: MouseEvent): void => {
    isResizingRef.current = true;
    document.body.style.userSelect = "none"; // Prevent text selection while resizing
    console.log("[useResizableElement] Started resizing");
  }, []);

  const stopResizing = useCallback((): void => {
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
    (e: MouseEvent): void => {
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
    if (!resizer) return;

    const handleMouseDown = (e: MouseEvent): void => {
      e.preventDefault();
      startResizing(e);
      document.addEventListener("mousemove", resize);
      document.addEventListener("mouseup", handleMouseUp);
    };

    const handleMouseUp = (): void => {
      document.removeEventListener("mousemove", resize);
      document.removeEventListener("mouseup", handleMouseUp);
      stopResizing();
    };

    resizer.addEventListener("mousedown", handleMouseDown as EventListener);

    return () => {
      resizer.removeEventListener(
        "mousedown",
        handleMouseDown as EventListener
      );
      document.removeEventListener("mousemove", resize);
      document.removeEventListener("mouseup", handleMouseUp);
    };
  }, [startResizing, resize, stopResizing]);

  // Whenever the element is destroyed or re-created, set its width from our ref
  useEffect(() => {
    if (elementRef.current) {
      if (currentWidthRef.current > 0) {
        elementRef.current.style.width = `${currentWidthRef.current}px`;
      } else {
        // If no width set, use default
        elementRef.current.style.width = `${defaultWidth}px`;
        currentWidthRef.current = defaultWidth;
        lastWidthRef.current = defaultWidth;
      }
    }
  }, [defaultWidth]);

  // Helper functions to save/restore width
  const saveCurrentWidth = useCallback((): number => {
    if (elementRef.current) {
      const width = parseInt(getComputedStyle(elementRef.current).width);
      if (width > 0) {
        lastWidthRef.current = width;
        return width;
      }
    }
    return lastWidthRef.current;
  }, []);

  const restoreWidth = useCallback((): void => {
    if (elementRef.current && lastWidthRef.current > 0) {
      elementRef.current.style.width = `${lastWidthRef.current}px`;
      currentWidthRef.current = lastWidthRef.current;
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
