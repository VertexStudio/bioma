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
export const md = new MarkdownIt({
  html: true,
  breaks: true,
  linkify: true,
  highlight: (code, lang) => {
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

    return `<pre class="language-text"><code>${md.utils.escapeHtml(
      code
    )}</code></pre>`;
  },
});

// Custom renderer for code blocks with header
const defaultFence = md.renderer.rules.fence.bind(md.renderer.rules);
md.renderer.rules.fence = (tokens, idx, options, env, slf) => {
  const token = tokens[idx];
  const code = token.content.trim();
  const lang = token.info.trim() || "";

  // Special handling for mermaid diagrams
  if (lang === "mermaid") {
    return `<div class="mermaid">${code}</div>`;
  }

  // Create wrapper for code block with header and copy button
  const rendered = defaultFence(tokens, idx, options, env, slf);
  return `
    <div class="code-block-wrapper">
      <div class="code-block-header">
        <span class="code-block-language">${lang || "text"}</span>
        <div class="code-block-actions">
          <button class="code-block-button copy-button" aria-label="Copy code">
            <i class="fas fa-copy"></i> Copy
          </button>
        </div>
      </div>
      ${rendered}
    </div>
  `;
};

// Function to process message content (for parsing special tags like SOURCE, IMAGE, etc.)
export const processMessageContent = (content, images = []) => {
  let processedContent = "";
  let normalLines = [];
  let imageIndex = 0;

  // Flush accumulated normal text
  function flushNormalLines() {
    if (normalLines.length) {
      const joined = normalLines.join("\n").trim();
      if (joined) {
        try {
          // Try to detect JSON and pretty-print it
          const jsonObj = JSON.parse(joined);
          processedContent += `<pre><code>${JSON.stringify(
            jsonObj,
            null,
            2
          )}</code></pre>`;
        } catch (e) {
          // Not valid JSON; render as Markdown
          processedContent += md.render(joined);
        }
      }
      normalLines = [];
    }
  }

  // Process each line for markers
  const lines = content.split("\n");
  lines.forEach((line) => {
    const trimmedLine = line.trim();
    if (
      trimmedLine.startsWith("[URI:") ||
      trimmedLine.startsWith("[SOURCE:") ||
      trimmedLine.startsWith("[CHUNK:")
    ) {
      flushNormalLines();
      // Keep the original format for display
      processedContent += `<div class="uri">${trimmedLine}</div>`;
    } else if (trimmedLine.startsWith("Source:")) {
      flushNormalLines();
      // Convert old "Source:" format to new [SOURCE:] format
      let source = trimmedLine.slice(7).trim();
      processedContent += `<div class="uri">[SOURCE:${source}]</div>`;
    } else if (trimmedLine.startsWith("[IMAGE:")) {
      flushNormalLines();
      let imageType = trimmedLine.slice(7, -1).trim();
      if (images && imageIndex < images.length) {
        let imageUrl = processBase64Image(images[imageIndex], imageType);
        if (imageUrl) {
          processedContent += `<div class="context-image"><img src="${imageUrl}" style="max-width: 100%; margin: 10px 0;"></div>`;
        }
        imageIndex++;
      } else {
        normalLines.push(line);
      }
    } else {
      normalLines.push(line);
    }
  });

  flushNormalLines();
  return processedContent;
};

// Helper to process a base64 image
export const processBase64Image = (base64String, type = "jpeg") => {
  base64String = base64String.trim();
  if (base64String.startsWith("data:image")) {
    return base64String;
  }
  try {
    atob(base64String); // verify Base64
    return `data:image/${type};base64,${base64String}`;
  } catch (e) {
    console.error("Invalid base64 string:", e);
    return null;
  }
};

// Function to initialize mermaid diagrams
export const initializeMermaid = () => {
  try {
    mermaid.contentLoaded();
  } catch (e) {
    console.error("Error initializing mermaid diagrams:", e);
  }
};

// Function to copy text to clipboard
export const copyToClipboard = async (text) => {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch (err) {
    console.error("Failed to copy text:", err);
    return false;
  }
};
