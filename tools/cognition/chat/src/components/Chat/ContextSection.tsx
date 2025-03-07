import React, { useState } from "react";
import {
  processMessageContent,
  copyToClipboard,
} from "../../utils/markdown.ts";
import { useChat } from "../../contexts/ChatContext";
import "./ContextSection.css";

interface ContextMessage {
  role: string;
  content: string;
  images?: Array<{ data: string; type: string }>;
}

interface ContextSectionProps {
  context: ContextMessage[] | null | undefined;
}

const ContextSection: React.FC<ContextSectionProps> = ({ context }) => {
  const [isExpanded, setIsExpanded] = useState<boolean>(false);
  const { ttftValue } = useChat();

  if (!context?.length) {
    return null;
  }

  const toggleContext = (): void => {
    setIsExpanded(!isExpanded);
  };

  const copyContext = async (): Promise<void> => {
    const textToCopy = context
      .filter((msg) => msg.role === "assistant")
      .map((msg) => msg.content)
      .join("\n\n");

    const success = await copyToClipboard(textToCopy);

    // Visual feedback could be added here
    if (success) {
      // Maybe show a toast notification
      console.log("Context copied to clipboard");
    }
  };

  // Process different message types in the context
  const regularMessages = context
    .filter((msg) => msg.role !== "system" && msg.role !== "user")
    .map((msg, idx) => {
      const roleClass = `${msg.role}-message`;
      const processedContent = processMessageContent(
        msg.content,
        msg.images || []
      );

      // Special handling for tool messages (JSON)
      if (msg.role === "tool") {
        let toolContent = msg.content || "";
        try {
          // Format JSON
          const jsonObj = JSON.parse(toolContent);
          toolContent = JSON.stringify(jsonObj, null, 2);

          return (
            <div key={`tool-${idx}`} className={`message ${roleClass}`}>
              <div className="tool-content">
                <pre>
                  <code>{toolContent}</code>
                </pre>
              </div>
            </div>
          );
        } catch (e) {
          // If not valid JSON, render as-is
          return (
            <div key={`tool-${idx}`} className={`message ${roleClass}`}>
              <div className="context-content">
                <pre>{msg.content}</pre>
              </div>
            </div>
          );
        }
      }

      // Regular message rendering
      return (
        <div key={`context-${idx}`} className={`message ${roleClass}`}>
          <div
            className="context-content"
            dangerouslySetInnerHTML={{ __html: processedContent }}
          />
        </div>
      );
    });

  // Find system messages
  const systemMessages = context
    .filter((msg) => msg.role === "system")
    .map((msg, idx) => (
      <div key={`system-${idx}`} className="message system-message">
        <div className="context-content">
          <pre>{msg.content}</pre>
        </div>
      </div>
    ));

  // Format time to first token if available
  const ttftFormatted = ttftValue ? `${(ttftValue / 1000).toFixed(2)}s` : null;

  return (
    <div className={`context-section ${isExpanded ? "expanded" : ""}`}>
      <div className="context-header" onClick={toggleContext}>
        <div className="context-title">
          <i className="fas fa-lightbulb"></i>
          <span>Context</span>
          {ttftFormatted && (
            <span className="ttft-label">TTFT: {ttftFormatted}</span>
          )}
        </div>
        <div className="context-controls">
          <button
            className="copy-context-button"
            onClick={(e) => {
              e.stopPropagation();
              copyContext();
            }}
            title="Copy context messages"
          >
            <i className="fas fa-copy"></i>
          </button>
          <i
            className={`context-toggle-icon fas fa-chevron-${
              isExpanded ? "up" : "down"
            }`}
          ></i>
        </div>
      </div>

      {isExpanded && (
        <div className="context-messages">
          {systemMessages}
          {regularMessages}
        </div>
      )}
    </div>
  );
};

export default ContextSection;
