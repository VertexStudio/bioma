import React, { useEffect } from 'react';
import { md, initializeMermaid } from '../../utils/markdown';
import './SystemMessage.css';

const SystemMessage = ({ content }) => {
  useEffect(() => {
    // Initialize mermaid diagrams if any
    initializeMermaid();
    
    // Highlight code blocks if any
    if (window.Prism) {
      window.Prism.highlightAll();
    }
  }, [content]);
  
  return (
    <div className="message system-message">
      <div 
        dangerouslySetInnerHTML={{ 
          __html: md.render(content) 
        }} 
      />
    </div>
  );
};

export default SystemMessage;