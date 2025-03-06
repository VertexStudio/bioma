// Calculate formatted duration from start time to now
export const calculateDuration = (startTime) => {
  if (!startTime) return "0.0s";
  const duration = ((Date.now() - startTime) / 1000).toFixed(1);
  return `${duration}s`;
};

// Resize a textarea to fit its content
export const resizeTextarea = (textareaRef) => {
  if (!textareaRef || !textareaRef.current) return;

  const textarea = textareaRef.current;
  textarea.style.height = "auto";
  textarea.style.height = `${textarea.scrollHeight}px`;
};

// Group array of objects by a specific key
export const groupBy = (array, key) => {
  return array.reduce((result, item) => {
    const groupKey = item[key];
    if (!result[groupKey]) {
      result[groupKey] = [];
    }
    result[groupKey].push(item);
    return result;
  }, {});
};

// Extract error messages from error objects or strings
export const extractErrorMessage = (error) => {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (error?.message) return error.message;
  return "An unknown error occurred";
};

// Truncate text with ellipsis
export const truncateText = (text, maxLength = 50) => {
  if (!text || text.length <= maxLength) return text;
  return `${text.substring(0, maxLength)}...`;
};

// Generate a unique ID
export const generateId = () => {
  return Date.now().toString(36) + Math.random().toString(36).substring(2);
};

// Debounce function to limit how often a function can be called
export const debounce = (func, wait) => {
  let timeout;
  return function executedFunction(...args) {
    const later = () => {
      clearTimeout(timeout);
      func(...args);
    };
    clearTimeout(timeout);
    timeout = setTimeout(later, wait);
  };
};

// Format milliseconds as human-readable time
export const formatTime = (ms) => {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(2)}s`;
};

// Check if an object is empty
export const isEmptyObject = (obj) => {
  return Object.keys(obj).length === 0;
};

// Safely parse JSON with a fallback value
export const safeJsonParse = (jsonString, fallback = null) => {
  try {
    return JSON.parse(jsonString);
  } catch (e) {
    console.error("JSON parse error:", e);
    return fallback;
  }
};
