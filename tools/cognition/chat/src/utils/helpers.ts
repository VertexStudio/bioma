import { RefObject } from "react";

// Calculate formatted duration from start time to now
export const calculateDuration = (startTime: Date | null): string => {
  if (!startTime) return "0.0s";
  const duration = ((Date.now() - startTime.getTime()) / 1000).toFixed(1);
  return `${duration}s`;
};

// Resize a textarea to fit its content
export const resizeTextarea = (
  textareaRef: RefObject<HTMLTextAreaElement>
): void => {
  if (!textareaRef || !textareaRef.current) return;

  const textarea = textareaRef.current;
  textarea.style.height = "auto";
  textarea.style.height = `${textarea.scrollHeight}px`;
};

// Group array of objects by a specific key
export const groupBy = <T extends Record<string, any>, K extends keyof T>(
  array: T[],
  key: K
): Record<string, T[]> => {
  return array.reduce((result: Record<string, T[]>, item: T) => {
    const groupKey = String(item[key]);
    if (!result[groupKey]) {
      result[groupKey] = [];
    }
    result[groupKey].push(item);
    return result;
  }, {});
};

// Extract error messages from error objects or strings
export const extractErrorMessage = (error: unknown): string => {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (error && typeof error === "object" && "message" in error) {
    return String((error as { message: unknown }).message);
  }
  return "An unknown error occurred";
};

// Truncate text with ellipsis
export const truncateText = (
  text: string | null | undefined,
  maxLength = 50
): string => {
  if (!text || text.length <= maxLength) return text || "";
  return `${text.substring(0, maxLength)}...`;
};

// Generate a unique ID
export const generateId = (): string => {
  return Date.now().toString(36) + Math.random().toString(36).substring(2);
};

// Debounce function to limit how often a function can be called
export const debounce = <T extends (...args: any[]) => any>(
  func: T,
  wait: number
): ((...args: Parameters<T>) => void) => {
  let timeout: ReturnType<typeof setTimeout> | null = null;

  return function executedFunction(...args: Parameters<T>): void {
    const later = () => {
      timeout = null;
      func(...args);
    };

    if (timeout) {
      clearTimeout(timeout);
    }
    timeout = setTimeout(later, wait);
  };
};

// Format milliseconds to readable time
export const formatTime = (ms: number): string => {
  const seconds = Math.floor(ms / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  return `${minutes}m ${seconds % 60}s`;
};

// Check if an object is empty
export const isEmptyObject = (
  obj: Record<string, any> | null | undefined
): boolean => {
  if (!obj) return true;
  return Object.keys(obj).length === 0;
};

// Safely parse JSON with fallback
export const safeJsonParse = <T>(
  jsonString: string,
  fallback: T | null = null
): T | null => {
  try {
    return JSON.parse(jsonString) as T;
  } catch (e) {
    return fallback;
  }
};
