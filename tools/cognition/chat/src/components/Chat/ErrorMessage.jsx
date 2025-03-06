import React, { useState } from 'react';
import { md } from '../../utils/markdown';
import './ErrorMessage.css';

const ErrorMessage = ({ error }) => {
  const [isExpanded, setIsExpanded] = useState(false);
  
  // If error is an array, we'll show multiple error items
  const errors = Array.isArray(error) ? error : [error];
  
  const toggleExpand = () => {
    setIsExpanded(!isExpanded);
  };
  
  return (
    <div className="message error-message">
      <div className="error-toggle" onClick={toggleExpand}>
        {isExpanded ? '▼' : '▶'}
      </div>
      <div className="error-header">
        ⚠️ {errors.length} Error{errors.length > 1 ? 's' : ''} Occurred
      </div>
      
      {isExpanded && (
        <div className="error-content">
          {errors.map((err, index) => (
            <div key={index} className="error-item">
              <i className="fas fa-exclamation-circle"></i>
              <div 
                dangerouslySetInnerHTML={{ 
                  __html: md.render(
                    typeof err === 'string' ? err : (err.message || 'Unknown error')
                  ) 
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