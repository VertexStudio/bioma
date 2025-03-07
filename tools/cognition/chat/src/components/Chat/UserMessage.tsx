import React, { useState, useRef, KeyboardEvent, ChangeEvent } from "react";
import { useChat } from "../../contexts/ChatContext";
import { md } from "../../utils/markdown.ts";
import { FaEdit } from "react-icons/fa";
import "./UserMessage.css";

interface UserMessageProps {
  content: string;
  index: number;
}

const UserMessage: React.FC<UserMessageProps> = ({ content, index }) => {
  const { editMessage } = useChat();
  const [isEditing, setIsEditing] = useState<boolean>(false);
  const [editedContent, setEditedContent] = useState<string>(content);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const handleEdit = (): void => {
    setIsEditing(true);
    setEditedContent(content);
    // Focus and select all text in the textarea after rendering
    setTimeout(() => {
      if (textareaRef.current) {
        textareaRef.current.focus();
        textareaRef.current.select();
      }
    }, 0);
  };

  const handleCancel = (): void => {
    setIsEditing(false);
  };

  const handleSave = (): void => {
    if (editedContent.trim()) {
      editMessage(index, editedContent);
      setIsEditing(false);
    }
  };

  // Handle Enter key to save and Escape key to cancel
  const handleKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>): void => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSave();
    } else if (e.key === "Escape") {
      handleCancel();
    }
  };

  // Adjust textarea height as content changes
  const handleTextareaChange = (e: ChangeEvent<HTMLTextAreaElement>): void => {
    setEditedContent(e.target.value);
    e.target.style.height = "auto";
    e.target.style.height = `${e.target.scrollHeight}px`;
  };

  return (
    <div className="message user-message">
      <div className="message-header">
        <span className="user-icon">👤</span>
        <span className="message-sender">You</span>

        {!isEditing && (
          <button
            className="edit-button"
            onClick={handleEdit}
            title="Edit message"
          >
            <FaEdit />
          </button>
        )}
      </div>

      {isEditing ? (
        <div className="message-edit">
          <textarea
            ref={textareaRef}
            value={editedContent}
            onChange={handleTextareaChange}
            onKeyDown={handleKeyDown}
            rows={1}
          />
          <div className="edit-actions">
            <button onClick={handleCancel}>Cancel</button>
            <button onClick={handleSave} disabled={!editedContent.trim()}>
              Save
            </button>
          </div>
        </div>
      ) : (
        <div
          className="message-content"
          dangerouslySetInnerHTML={{
            __html: md.render(content),
          }}
        />
      )}
    </div>
  );
};

export default UserMessage;
