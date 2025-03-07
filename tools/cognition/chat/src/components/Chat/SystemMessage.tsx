import React, { useEffect } from "react";
import { md, initializeMermaid } from "../../utils/markdown.ts";
import Prism from "prismjs";
import "./SystemMessage.css";

interface SystemMessageProps {
  content: string;
}

const SystemMessage: React.FC<SystemMessageProps> = ({ content }) => {
  useEffect(() => {
    // Initialize mermaid diagrams if any
    initializeMermaid();

    // Highlight code blocks if any
    if (typeof window !== "undefined" && Prism) {
      Prism.highlightAll();
    }
  }, [content]);

  return (
    <div className="message system-message">
      <div
        dangerouslySetInnerHTML={{
          __html: md.render(content),
        }}
      />
    </div>
  );
};

export default SystemMessage;
