import React, { useEffect } from 'react';
import { useChat } from '../../contexts/ChatContext';
import { useConfig } from '../../contexts/ConfigContext';
import UserMessage from './UserMessage';
import AssistantMessage from './AssistantMessage';
import SystemMessage from './SystemMessage';
import ToolMessage from './ToolMessage';
import ErrorMessage from './ErrorMessage';
import ThinkingProcess from './ThinkingProcess';
import './ChatContainer.css';

const ChatContainer = () => {
  const { 
    messageHistory, 
    chatContainerRef, 
    isThinkingActive,
    thinkingContent,
    thinkingStartTime,
    scrollToBottom 
  } = useChat();
  
  const { sidebarVisible } = useConfig();
  
  // Scroll to bottom when messages are added
  useEffect(() => {
    scrollToBottom();
  }, [messageHistory, scrollToBottom]);
  
  return (
    <div 
      id="chat-container" 
      ref={chatContainerRef}
      className={sidebarVisible ? '' : 'sidebar-collapsed'}
    >
      {messageHistory.map((message, index) => {
        switch (message.role) {
          case 'user':
            return (
              <UserMessage 
                key={`user-${index}`} 
                content={message.content} 
                index={index}
              />
            );
          case 'assistant':
            return (
              <AssistantMessage 
                key={`assistant-${index}`} 
                content={message.content}
                context={message.context}
                index={index}
              />
            );
          case 'system':
            return (
              <SystemMessage 
                key={`system-${index}`} 
                content={message.content} 
              />
            );
          case 'tool':
            return (
              <ToolMessage 
                key={`tool-${index}`} 
                content={message.content} 
              />
            );
          case 'error':
            return (
              <ErrorMessage 
                key={`error-${index}`} 
                error={message.content} 
              />
            );
          default:
            return null;
        }
      })}
      
      {/* Show thinking process if active */}
      {isThinkingActive && (
        <ThinkingProcess 
          content={thinkingContent}
          startTime={thinkingStartTime}
        />
      )}
    </div>
  );
};

export default ChatContainer;