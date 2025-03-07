// Import without type checking for MarkdownIt specifically
// @ts-ignore
import MarkdownIt from "markdown-it";
import Prism from "prismjs";
import mermaid from "mermaid";

// Initialize mermaid
mermaid.initialize({
  startOnLoad: true,
  theme: "dark",
  securityLevel: "loose",
});

// Create and configure markdown-it instance
export const md = MarkdownIt({
  html: true,
  breaks: true,
  linkify: true,
  highlight: (code: string, lang: string): string => {
    if (lang === "mermaid") {
      return `<div class="mermaid">${code}</div>`;
    }

    if (lang && Prism.languages[lang]) {
      try {
        return `<pre class="language-${lang}"><code>${Prism.highlight(
          code,
          Prism.languages[lang],
          lang
        )}</code></pre>`;
      } catch (e) {
        console.error(`Error highlighting code block: ${e}`);
      }
    }

    // Use default escaping for unknown languages
    return `<pre class="language-${lang}"><code>${md.utils.escapeHtml(
      code
    )}</code></pre>`;
  },
});

// Add plugins and extensions
// TODO: Add TypeScript typings for any plugins if available

interface ImageData {
  data: string;
  type: string;
}

// Process message content with image handling
export const processMessageContent = (
  content: string,
  images: ImageData[] = []
): string => {
  if (!content) return "";

  // If no images, just return the rendered markdown
  if (!images || images.length === 0) {
    return md.render(content);
  }

  // Process content with image placeholders
  const lines = content.split("\n");
  let normalLines: string[] = [];
  let result = "";

  function flushNormalLines(): void {
    if (normalLines.length > 0) {
      result += md.render(normalLines.join("\n"));
      normalLines = [];
    }
  }

  lines.forEach((line) => {
    // Check if line contains an image reference
    const imgMatch = line.match(/!\[([^\]]*)\]\(data:([^;)]+);([^)]+)\)/);
    if (imgMatch) {
      // Flush any pending normal lines
      flushNormalLines();

      // Find the matching image in the array
      const imageType = imgMatch[2];
      const imageRef = imgMatch[3];
      const altText = imgMatch[1] || "image";

      if (imageRef.startsWith("ref:")) {
        const imageIndex = parseInt(imageRef.split(":")[1], 10);
        if (images[imageIndex]) {
          const imgData = images[imageIndex].data;
          const imgType = images[imageIndex].type || imageType || "jpeg";
          result += `<div class="message-image-container">
            <img src="data:image/${imgType};base64,${imgData}" alt="${altText}" />
          </div>`;
        } else {
          result += `<div class="message-image-error">Image reference not found</div>`;
        }
      } else {
        result += `<div class="message-image-error">Invalid image reference</div>`;
      }
    } else if (line.trim().startsWith("```mermaid")) {
      // Handle mermaid diagrams specially
      flushNormalLines();
      // Add the line to start a new section
      normalLines.push(line);
    } else if (
      line.trim() === "```" &&
      normalLines.some((l) => l.trim().startsWith("```mermaid"))
    ) {
      // End of mermaid diagram
      normalLines.push(line);
      flushNormalLines();
    } else {
      // Regular line, add to normal lines
      normalLines.push(line);
    }
  });

  // Flush any remaining normal lines
  flushNormalLines();

  return result;
};

// Process and convert base64 string
export const processBase64Image = (
  base64String: string,
  type = "jpeg"
): ImageData => {
  // Remove data URL prefix if present
  const base64Data = base64String.includes("base64,")
    ? base64String.split("base64,")[1]
    : base64String;

  return {
    data: base64Data,
    type,
  };
};

// Initialize mermaid for rendering diagrams
export const initializeMermaid = (): void => {
  try {
    // The timeout helps ensure DOM elements are ready
    setTimeout(() => {
      mermaid.init(undefined, document.querySelectorAll(".mermaid"));
    }, 0);
  } catch (e) {
    console.error("Error initializing mermaid:", e);
  }
};

// Copy text to clipboard
export const copyToClipboard = async (text: string): Promise<boolean> => {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch (e) {
    console.error("Failed to copy to clipboard:", e);
    // Fallback method for older browsers
    try {
      const textArea = document.createElement("textarea");
      textArea.value = text;
      document.body.appendChild(textArea);
      textArea.select();
      document.execCommand("copy");
      document.body.removeChild(textArea);
      return true;
    } catch (fallbackError) {
      console.error("Fallback clipboard copy failed:", fallbackError);
      return false;
    }
  }
};
