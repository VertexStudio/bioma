import React, { useRef, useEffect, MouseEvent, KeyboardEvent } from "react";
import { useChat } from "../../contexts/ChatContext";
import { useConfig } from "../../contexts/ConfigContext";
import { resizeTextarea } from "../../utils/helpers.ts";
import UtilitiesContainer from "./UtilitiesContainer.tsx";
import "./InputContainer.css";

const InputContainer: React.FC = () => {
  const {
    inputValue,
    setInputValue,
    handleSubmit,
    isGenerating,
    stopGeneration,
  } = useChat();

  const { sidebarVisible } = useConfig();

  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // Auto-resize textarea as content changes
  useEffect(() => {
    if (textareaRef.current) {
      resizeTextarea(textareaRef as React.RefObject<HTMLTextAreaElement>);
    }
  }, [inputValue]);

  // Handle input clicks (focus textarea for better UX)
  const handleContainerClick = (e: MouseEvent<HTMLDivElement>): void => {
    // If clicking the textarea or a button, don't interfere
    if (
      e.target === textareaRef.current ||
      (e.target instanceof Element && e.target.closest("button"))
    ) {
      return;
    }

    // Focus the textarea and place cursor at the end
    if (textareaRef.current) {
      textareaRef.current.focus();
      const len = textareaRef.current.value.length;
      textareaRef.current.setSelectionRange(len, len);
    }
  };

  // Handle key presses in textarea
  const handleKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>): void => {
    if (e.key === "Enter") {
      if (e.shiftKey) {
        // Allow line breaks with Shift+Enter
        return;
      }

      if (e.ctrlKey || e.altKey) {
        // Ctrl+Enter and Alt+Enter are handled in UtilitiesContainer
        return;
      }

      // Regular Enter submits the form
      e.preventDefault();
      if (!isGenerating && inputValue.trim()) {
        handleSubmit();
      }
    } else if (e.key === "Escape") {
      // Escape stops generation
      if (isGenerating) {
        stopGeneration();
      }
    }
  };

  return (
    <div
      id="input-container"
      className={sidebarVisible ? "" : "sidebar-collapsed"}
      onClick={handleContainerClick}
    >
      <textarea
        id="query-input"
        ref={textareaRef}
        value={inputValue}
        onChange={(e) => setInputValue(e.target.value)}
        onKeyDown={handleKeyDown}
        placeholder="How can I help you today?"
        rows={1}
        disabled={isGenerating}
      />

      <UtilitiesContainer />
    </div>
  );
};

export default InputContainer;
