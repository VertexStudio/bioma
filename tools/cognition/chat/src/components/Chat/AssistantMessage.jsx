import React, { useEffect, useState } from 'react';
import { initializeMermaid, processMessageContent } from '../../utils/markdown';
import ContextSection from './ContextSection';
import './AssistantMessage.css';

const AssistantMessage = ({ content, context, index }) => {
  const [isExpanded, setIsExpanded] = useState(true);
  
  // Initialize mermaid diagrams after rendering
  useEffect(() => {
    initializeMermaid();
    
    // Initialize Prism.js syntax highlighting
    if (window.Prism) {
      window.Prism.highlightAll();
    }
    
    // Add event listeners to all copy buttons
    const copyButtons = document.querySelectorAll('.copy-button');
    copyButtons.forEach(button => {
      if (!button.hasListener) {
        button.addEventListener('click', handleCopyCode);
        button.hasListener = true;
      }
    });
    
    return () => {
      // Clean up copy button event listeners
      copyButtons.forEach(button => {
        button.removeEventListener('click', handleCopyCode);
      });
    };
  }, [content]);
  
  // Handle code copying
  const handleCopyCode = async (e) => {
    const button = e.currentTarget;
    const codeBlock = button.closest('.code-block-wrapper');
    const code = codeBlock.querySelector('code');
    
    try {
      await navigator.clipboard.writeText(code.innerText);
      button.innerHTML = '<i class="fas fa-check"></i> Copied';
      button.classList.add('copied');
      setTimeout(() => {
        button.innerHTML = '<i class="fas fa-copy"></i> Copy';
        button.classList.remove('copied');
      }, 2000);
    } catch (err) {
      console.error('Failed to copy text:', err);
      button.innerHTML = '<i class="fas fa-times"></i> Failed';
      setTimeout(() => {
        button.innerHTML = '<i class="fas fa-copy"></i> Copy';
      }, 2000);
    }
  };
  
  // Process the content to handle special tags
  const processedContent = processMessageContent(content);
  
  return (
    <div className={`message assistant-message ${isExpanded ? 'expanded' : ''}`} data-message-index={index}>
      <div 
        className="assistant-content"
        dangerouslySetInnerHTML={{ __html: processedContent }}
      />
      
      {/* Context section will show sources, etc. */}
      {context && <ContextSection context={context} />}
    </div>
  );
};

export default AssistantMessage;