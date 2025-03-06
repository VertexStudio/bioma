import React, { useState, useEffect } from 'react';
import { calculateDuration } from '../../utils/helpers';
import './ThinkingProcess.css';

const ThinkingProcess = ({ content, startTime }) => {
  const [isExpanded, setIsExpanded] = useState(false);
  const [duration, setDuration] = useState('');
  const [isThinking, setIsThinking] = useState(true);
  
  // Calculate and update thinking duration
  useEffect(() => {
    if (!startTime) return;
    
    // Initial duration calculation
    setDuration(calculateDuration(startTime));
    
    // Set up interval to update duration every 100ms
    const intervalId = setInterval(() => {
      setDuration(calculateDuration(startTime));
    }, 100);
    
    return () => clearInterval(intervalId);
  }, [startTime]);
  
  // When content changes, check if thinking is over
  useEffect(() => {
    if (content && !content.trim().endsWith('...')) {
      setIsThinking(false);
    }
  }, [content]);
  
  const toggleThinking = () => {
    setIsExpanded(!isExpanded);
  };
  
  return (
    <div className="thinking-process">
      <div 
        className={`thinking-header ${isExpanded ? 'expanded' : ''}`}
        onClick={toggleThinking}
      >
        <i className="fas fa-chevron-right"></i>
        <span className={`thinking-text ${!isThinking ? 'static' : ''}`}>
          {isThinking 
            ? `Thinking...` 
            : `Thought for ${duration}`
          }
        </span>
      </div>
      <div className={`thinking-content ${isExpanded ? 'visible' : ''}`}>
        {content}
      </div>
    </div>
  );
};

export default ThinkingProcess;