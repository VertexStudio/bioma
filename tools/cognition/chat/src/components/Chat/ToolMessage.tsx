import React, { useState, useEffect } from "react";
import "./ToolMessage.css";

interface ToolData {
  tool?: string;
  call?: {
    function?: {
      arguments?: Record<string, any>;
    };
  };
  response?: any;
}

interface ToolMessageProps {
  content: string | ToolData;
}

const ToolMessage: React.FC<ToolMessageProps> = ({ content }) => {
  const [isExpanded, setIsExpanded] = useState<boolean>(false);
  const [toolData, setToolData] = useState<ToolData | null>(null);

  useEffect(() => {
    // Parse the tool data from content
    try {
      const data = typeof content === "string" ? JSON.parse(content) : content;
      setToolData(data);
    } catch (error) {
      console.error("Error parsing tool data:", error);
    }
  }, [content]);

  const toggleExpand = (): void => {
    setIsExpanded(!isExpanded);
  };

  if (!toolData) {
    return (
      <div className="message tool-message">
        <div className="tool-header">🔧 Tool Call: Error parsing tool data</div>
      </div>
    );
  }

  return (
    <div className="message tool-message">
      <div className="tool-toggle" onClick={toggleExpand}>
        {isExpanded ? "▼" : "▶"}
      </div>
      <div className="tool-header">
        🔧 Tool Call: {toolData.tool || "Unknown Tool"}
      </div>

      {isExpanded && (
        <div className="tool-content">
          <div className="tool-details">
            <strong>Arguments:</strong>
            <pre>
              <code>
                {JSON.stringify(
                  toolData.call?.function?.arguments || {},
                  null,
                  2
                )}
              </code>
            </pre>

            <strong>Response:</strong>
            <pre>
              <code>{JSON.stringify(toolData.response || {}, null, 2)}</code>
            </pre>
          </div>
        </div>
      )}
    </div>
  );
};

export default ToolMessage;
