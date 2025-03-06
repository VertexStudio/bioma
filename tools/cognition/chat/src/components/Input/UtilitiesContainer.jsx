import React, { useEffect } from 'react';
import { useChat } from '../../contexts/ChatContext';
import { useConfig } from '../../contexts/ConfigContext';
import { FaArrowUp, FaStop, FaBrain, FaWrench } from 'react-icons/fa';
import './UtilitiesContainer.css';

const UtilitiesContainer = () => {
  const { 
    inputValue,
    handleSubmit,
    isGenerating,
    stopGeneration
  } = useChat();
  
  const { 
    toolsEnabled,
    thinkEnabled,
    toggleTools,
    toggleThink,
    activeTools
  } = useConfig();
  
  // Add keyboard shortcuts
  useEffect(() => {
    const handleKeyDown = (e) => {
      // Ctrl+Enter toggles tools mode
      if (e.key === 'Enter' && e.ctrlKey) {
        e.preventDefault();
        if (activeTools.length > 0) {
          toggleTools();
        }
      }
      
      // Alt+Enter toggles think mode
      if (e.key === 'Enter' && e.altKey) {
        e.preventDefault();
        toggleThink();
      }
    };
    
    document.addEventListener('keydown', handleKeyDown);
    
    return () => {
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, [toggleTools, toggleThink, activeTools]);
  
  const handleSendClick = () => {
    if (!isGenerating && inputValue.trim()) {
      handleSubmit();
    }
  };
  
  const hasActiveTools = activeTools.length > 0;
  
  return (
    <div className="utilities-container">
      <div className="utilities-left">
        <button
          id="send-button"
          className={`action-button ${toolsEnabled ? 'active' : ''}`}
          title="Toggle tools mode (Ctrl+Enter)"
          onClick={toggleTools}
          disabled={!hasActiveTools}
          style={!hasActiveTools ? {
            opacity: '0.5',
            cursor: 'not-allowed'
          } : {}}
        >
          <FaWrench />
        </button>
        
        <button
          id="think-button"
          className={`action-button ${thinkEnabled ? 'active' : ''}`}
          title="Toggle think mode (Alt+Enter)"
          onClick={toggleThink}
        >
          <FaBrain />
        </button>
      </div>
      
      <div className="utilities-right">
        {isGenerating ? (
          <button
            id="stop-button"
            title="Stop generation (Esc)"
            onClick={stopGeneration}
          >
            <FaStop />
          </button>
        ) : (
          <button
            className="send-button"
            title="Send message"
            onClick={handleSendClick}
            disabled={!inputValue.trim()}
          >
            <FaArrowUp />
          </button>
        )}
      </div>
    </div>
  );
};

export default UtilitiesContainer;