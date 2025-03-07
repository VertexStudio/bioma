import React, { useState } from "react";
import { md } from "../../utils/markdown.ts";
import "./ErrorMessage.css";

interface ErrorMessageProps {
  error: string | Error | Array<string | Error>;
}

const ErrorMessage: React.FC<ErrorMessageProps> = ({ error }) => {
  const [isExpanded, setIsExpanded] = useState<boolean>(false);

  // If error is an array, we'll show multiple error items
  const errors = Array.isArray(error) ? error : [error];

  const toggleExpand = (): void => {
    setIsExpanded(!isExpanded);
  };

  return (
    <div className="message error-message">
      <div className="error-toggle" onClick={toggleExpand}>
        {isExpanded ? "▼" : "▶"}
      </div>
      <div className="error-header">
        ⚠️ {errors.length} Error{errors.length > 1 ? "s" : ""} Occurred
      </div>

      {isExpanded && (
        <div className="error-content">
          {errors.map((err, index) => (
            <div key={index} className="error-item">
              <i className="fas fa-exclamation-circle"></i>
              <div
                dangerouslySetInnerHTML={{
                  __html: md.render(
                    typeof err === "string"
                      ? err
                      : (err as Error).message || "Unknown error"
                  ),
                }}
              />
            </div>
          ))}
        </div>
      )}
    </div>
  );
};

export default ErrorMessage;
