import React, { useState } from 'react';
import { processMessageContent } from '../../utils/markdown';
import { useChat } from '../../contexts/ChatContext';
import { copyToClipboard } from '../../utils/markdown';
import './ContextSection.css';

const ContextSection = ({ context }) => {
  const [isExpanded, setIsExpanded] = useState(false);
  const { ttftValue } = useChat();
  
  if (!context?.length) {
    return null;
  }
  
  const toggleContext = () => {
    setIsExpanded(!isExpanded);
  };
  
  const copyContext = async () => {
    const textToCopy = context
      .filter(msg => msg.role === 'assistant')
      .map(msg => msg.content)
      .join('\n\n');
    
    const success = await copyToClipboard(textToCopy);
    
    // Visual feedback could be added here
    if (success) {
      // Maybe show a toast notification
      console.log('Context copied to clipboard');
    }
  };
  
  // Process different message types in the context
  const regularMessages = context
    .filter(msg => msg.role !== 'system' && msg.role !== 'user')
    .map((msg, idx) => {
      const roleClass = `${msg.role}-message`;
      const processedContent = processMessageContent(msg.content, msg.images || []);
      
      // Special handling for tool messages (JSON)
      if (msg.role === 'tool') {
        let toolContent = msg.content || '';
        try {
          // Format JSON
          const jsonObj = JSON.parse(toolContent);
          toolContent = JSON.stringify(jsonObj, null, 2);
          
          return (
            <div key={`tool-${idx}`} className={`message ${roleClass}`}>
              <div className="chunk">
                <pre><code>{toolContent}</code></pre>
              </div>
            </div>
          );
        } catch (e) {
          // Not valid JSON, use regular processing
          return (
            <div key={`msg-${idx}`} className={`message ${roleClass}`}>
              <div className="chunk" dangerouslySetInnerHTML={{ __html: processedContent }} />
            </div>
          );
        }
      }
      
      return (
        <div key={`msg-${idx}`} className={`message ${roleClass}`}>
          <div className="chunk" dangerouslySetInnerHTML={{ __html: processedContent }} />
        </div>
      );
    });
  
  // Process system messages
  const systemMessages = context
    .filter(msg => msg.role === 'system')
    .map((msg, idx) => {
      const roleClass = 'system-message';
      let systemContent = msg.content.replace(
        /^\s*#{1,6}\s*ADDITIONAL CONTEXT:/im,
        'ADDITIONAL CONTEXT:'
      );
      const processedContent = processMessageContent(systemContent, msg.images || []);
      
      return (
        <div key={`system-${idx}`} className={`message ${roleClass}`}>
          <strong>{msg.role.charAt(0).toUpperCase() + msg.role.slice(1)}:</strong>
          <div className="chunk" dangerouslySetInnerHTML={{ __html: processedContent }} />
        </div>
      );
    });
  
  // Process the last user message
  const lastUserIndex = [...context].reverse().findIndex(msg => msg.role === 'user');
  const lastMessageIndex = lastUserIndex !== -1 ? context.length - 1 - lastUserIndex : -1;
  
  let lastUserMessage = null;
  if (lastMessageIndex !== -1) {
    const lastMsg = context[lastMessageIndex];
    const roleClass = `${lastMsg.role}-message`;
    const processedContent = processMessageContent(lastMsg.content || '', lastMsg.images || []);
    
    lastUserMessage = (
      <div className={`message ${roleClass}`}>
        <strong>{lastMsg.role.charAt(0).toUpperCase() + lastMsg.role.slice(1)}:</strong>
        <div className="chunk" dangerouslySetInnerHTML={{ __html: processedContent }} />
      </div>
    );
  }
  
  // Check if we need separators
  const hasRegularMessages = regularMessages.length > 0;
  const hasSystemMessages = systemMessages.length > 0;
  const hasLastUserMessage = lastUserMessage !== null;
  
  return (
    <div className="context-container">
      <div className="context-actions">
        <div className="context-toggle" onClick={toggleContext}>
          {isExpanded ? 'Hide Context ▼' : 'Show Context ▶'}
        </div>
        <div className="context-right">
          {ttftValue && (
            <div className="ttft-display" title="Time to first token">
              ⏱️ {(ttftValue / 1000).toFixed(2)}s
            </div>
          )}
          <div 
            className="context-copy" 
            onClick={copyContext} 
            title="Copy raw response"
          >
            <i className="fas fa-copy"></i>
          </div>
        </div>
      </div>
      
      {isExpanded && (
        <div className="context-content">
          {hasRegularMessages && regularMessages}
          
          {hasRegularMessages && hasSystemMessages && (
            <div className="source-separator"></div>
          )}
          
          {hasSystemMessages && systemMessages}
          
          {(hasRegularMessages || hasSystemMessages) && hasLastUserMessage && (
            <div className="source-separator"></div>
          )}
          
          {hasLastUserMessage && lastUserMessage}
        </div>
      )}
    </div>
  );
};

export default ContextSection;