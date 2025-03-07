import React, { useEffect, useState, MouseEvent } from "react";
import {
  initializeMermaid,
  processMessageContent,
} from "../../utils/markdown.ts";
import Prism from "prismjs";
import ContextSection from "./ContextSection.tsx";
import "./AssistantMessage.css";

interface AssistantMessageProps {
  content: string;
  context?: any;
  index: number;
}

const AssistantMessage: React.FC<AssistantMessageProps> = ({
  content,
  context,
  index,
}) => {
  const [isExpanded, setIsExpanded] = useState<boolean>(true);

  // Initialize mermaid diagrams after rendering
  useEffect(() => {
    initializeMermaid();

    // Initialize Prism.js syntax highlighting
    if (typeof window !== "undefined" && Prism) {
      Prism.highlightAll();
    }

    // Add event listeners to all copy buttons
    const copyButtons = document.querySelectorAll(".copy-button");
    copyButtons.forEach((button) => {
      if (!(button as any).hasListener) {
        button.addEventListener("click", handleCopyCode);
        (button as any).hasListener = true;
      }
    });

    return () => {
      // Clean up copy button event listeners
      copyButtons.forEach((button) => {
        button.removeEventListener("click", handleCopyCode);
      });
    };
  }, [content]);

  // Handle code copying
  const handleCopyCode = async (e: Event): Promise<void> => {
    const button = e.currentTarget as HTMLElement;
    const codeBlock = button.closest(".code-block-wrapper") as HTMLElement;
    const code = codeBlock.querySelector("code") as HTMLElement;

    try {
      await navigator.clipboard.writeText(code.innerText);
      button.innerHTML = '<i class="fas fa-check"></i> Copied';
      button.classList.add("copied");
      setTimeout(() => {
        button.innerHTML = '<i class="fas fa-copy"></i> Copy';
        button.classList.remove("copied");
      }, 2000);
    } catch (err) {
      console.error("Failed to copy text:", err);
    }
  };

  return (
    <div className="message assistant-message">
      <div className="message-header">
        <span className="assistant-icon">🤖</span>
        <span className="message-sender">AI Assistant</span>
      </div>

      <div className="message-content">
        <div
          dangerouslySetInnerHTML={{
            __html: processMessageContent(content),
          }}
        />
      </div>

      {context && <ContextSection context={context} />}
    </div>
  );
};

export default AssistantMessage;
