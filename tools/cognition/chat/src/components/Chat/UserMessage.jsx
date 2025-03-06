import React, { useState, useRef } from 'react';
import { useChat } from '../../contexts/ChatContext';
import { md } from '../../utils/markdown';
import { FaEdit } from 'react-icons/fa';
import './UserMessage.css';

const UserMessage = ({ content, index }) => {
  const { editMessage } = useChat();
  const [isEditing, setIsEditing] = useState(false);
  const [editedContent, setEditedContent] = useState(content);
  const textareaRef = useRef(null);
  
  const handleEdit = () => {
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
  
  const handleCancel = () => {
    setIsEditing(false);
  };
  
  const handleSave = () => {
    if (editedContent.trim()) {
      editMessage(index, editedContent);
      setIsEditing(false);
    }
  };
  
  // Handle Enter key to save and Escape key to cancel
  const handleKeyDown = (e) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSave();
    } else if (e.key === 'Escape') {
      handleCancel();
    }
  };
  
  // Adjust textarea height as content changes
  const handleTextareaChange = (e) => {
    setEditedContent(e.target.value);
    e.target.style.height = 'auto';
    e.target.style.height = `${e.target.scrollHeight}px`;
  };
  
  if (isEditing) {
    return (
      <div className="message user-message editing">
        <div className="edit-container">
          <textarea
            ref={textareaRef}
            value={editedContent}
            onChange={handleTextareaChange}
            onKeyDown={handleKeyDown}
            className="edit-textarea"
          />
          <div className="edit-buttons">
            <button onClick={handleCancel} className="edit-button cancel">
              Cancel
            </button>
            <button onClick={handleSave} className="edit-button save">
              Send
            </button>
          </div>
        </div>
      </div>
    );
  }
  
  return (
    <div className="message user-message">
      <div 
        dangerouslySetInnerHTML={{ 
          __html: md.render(content) 
        }} 
      />
      <div className="edit-icon" onClick={handleEdit}>
        <FaEdit />
      </div>
    </div>
  );
};

export default UserMessage;