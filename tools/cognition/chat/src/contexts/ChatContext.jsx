import React, { createContext, useState, useContext, useRef } from 'react';
import { useConfig } from './ConfigContext';
import { useRAGService } from '../hooks/useRAGService';

const ChatContext = createContext();

export const useChat = () => useContext(ChatContext);

export const ChatProvider = ({ children }) => {
  const { toolsEnabled, thinkEnabled, activeTools, activeSources } = useConfig();
  
  // Chat state
  const [messageHistory, setMessageHistory] = useState([]);
  const [inputValue, setInputValue] = useState('');
  const [isGenerating, setIsGenerating] = useState(false);
  const [assistantTextBuffer, setAssistantTextBuffer] = useState('');
  const [latestContext, setLatestContext] = useState(null);
  const [contextSet, setContextSet] = useState(false);
  const [ttftValue, setTtftValue] = useState(null);
  
  // Thinking process state
  const [isThinkingActive, setIsThinkingActive] = useState(false);
  const [thinkingContent, setThinkingContent] = useState('');
  const [thinkingStartTime, setThinkingStartTime] = useState(null);
  const [thinkingFromEndpoint, setThinkingFromEndpoint] = useState(false);
  
  // References
  const chatContainerRef = useRef(null);
  const controllerRef = useRef(null);
  
  const { sendQuery, abortQuery } = useRAGService({
    onResponse: handleResponse,
    onError: handleError,
    onComplete: handleComplete
  });

  // Add a new message to the chat
  const addMessage = (role, content, context = null) => {
    const newMessage = { role, content };
    setMessageHistory(prev => [...prev, newMessage]);
    
    if (role === 'assistant' && context) {
      setLatestContext(context);
    }
  };

  // Handle chat submission
  const handleSubmit = async (override = null) => {
    if (isGenerating && !override) return;
    
    const query = override || inputValue.trim();
    if (!query) return;
    
    // Set up for new generation
    setIsGenerating(true);
    setAssistantTextBuffer('');
    setContextSet(false);
    setLatestContext(null);
    setTtftValue(null);
    
    // Clear input unless we're using an override (edit case)
    if (!override) {
      addMessage('user', query);
      setInputValue('');
    }
    
    // Create thinking container if using think endpoint
    if (thinkEnabled) {
      setIsThinkingActive(true);
      setThinkingContent('');
      setThinkingStartTime(Date.now());
      setThinkingFromEndpoint(true);
    } else {
      // Create empty assistant message for regular chat
      addMessage('assistant', '');
    }
    
    // Get active tools
    const toolsList = toolsEnabled ? activeTools.map(tool => tool.actor) : [];
    
    // Get active sources
    const sourcesList = activeSources.map(source => source.path);
    
    // Send query to RAG server
    const endpoint = thinkEnabled ? '/think' : '/chat';
    await sendQuery(endpoint, messageHistory, toolsList, sourcesList);
  };

  // Edit a user message and regenerate response
  const editMessage = (index, newContent) => {
    if (isGenerating) {
      abortQuery();
    }
    
    // Update the specified message
    setMessageHistory(prev => {
      const newHistory = [...prev.slice(0, index + 1)];
      newHistory[index] = { ...newHistory[index], content: newContent };
      return newHistory;
    });
    
    // Submit with the edited content
    handleSubmit(newContent);
  };

  // Handle response from RAG server
  function handleResponse(data) {
    if (data.ttft_ms !== undefined) {
      setTtftValue(data.ttft_ms);
    }
    
    if (data.context) {
      setLatestContext(data.context);
    }
    
    if (!data.message?.content) {
      return;
    }
    
    const { role, content } = data.message;
    
    if (role === 'system' || role === 'tool') {
      // Handle special messages (system and tool)
      addMessage(role, content);
      return;
    }
    
    if (role === 'assistant') {
      const chunk = content;
      
      // Check if this is a thinking message
      const isThinkingMessage = chunk.startsWith('<think>');
      
      if (isThinkingMessage) {
        if (!isThinkingActive) {
          setIsThinkingActive(true);
          setThinkingContent('');
          setThinkingStartTime(Date.now());
          setThinkingFromEndpoint(false);
        }
      } else if (isThinkingActive && thinkingFromEndpoint && !chunk.includes('<think>')) {
        // If we get regular content in think mode, switch to normal message
        setIsThinkingActive(false);
        setThinkingFromEndpoint(false);
        
        const newBuffer = assistantTextBuffer + chunk;
        setAssistantTextBuffer(newBuffer);
        return;
      }
      
      // Process the text with thinking tags
      let idx = 0;
      let newAssistantBuffer = assistantTextBuffer;
      let newThinkingContent = thinkingContent;
      let newIsThinkingActive = isThinkingActive;
      
      while (idx < chunk.length) {
        if (!newIsThinkingActive) {
          const openPos = chunk.indexOf('<think>', idx);
          if (openPos === -1) {
            // No more thinking tags
            newAssistantBuffer += chunk.substring(idx);
            idx = chunk.length;
          } else {
            // Found opening thinking tag
            const visiblePart = chunk.substring(idx, openPos);
            if (visiblePart.trim()) {
              newAssistantBuffer += visiblePart;
            }
            idx = openPos + 7; // length of <think>
            newIsThinkingActive = true;
            setThinkingStartTime(Date.now());
            setThinkingFromEndpoint(false);
          }
        } else {
          const closePos = chunk.indexOf('</think>', idx);
          if (closePos === -1) {
            // No closing tag yet
            newThinkingContent += chunk.substring(idx);
            idx = chunk.length;
          } else {
            // Found closing thinking tag
            const thoughtChunk = chunk.substring(idx, closePos);
            if (thoughtChunk.trim()) {
              newThinkingContent += thoughtChunk;
            }
            idx = closePos + 8; // length of </think>
            newIsThinkingActive = false;
          }
        }
      }
      
      // Update state
      setAssistantTextBuffer(newAssistantBuffer);
      setThinkingContent(newThinkingContent);
      setIsThinkingActive(newIsThinkingActive);
    }
  }
  
  // Handle error from RAG server
  function handleError(error) {
    console.error('Error from RAG server:', error);
    setIsGenerating(false);
    
    // Reset thinking state if active
    if (isThinkingActive) {
      setIsThinkingActive(false);
      setThinkingFromEndpoint(false);
    }
  }
  
  // Handle completion of request
  function handleComplete() {
    if (assistantTextBuffer) {
      addMessage('assistant', assistantTextBuffer, latestContext);
    }
    
    setIsGenerating(false);
    
    // Reset thinking state if active
    if (isThinkingActive) {
      const duration = ((Date.now() - thinkingStartTime) / 1000).toFixed(1);
      setIsThinkingActive(false);
      setThinkingFromEndpoint(false);
    }
  }
  
  // Stop the generation process
  const stopGeneration = () => {
    if (isGenerating) {
      abortQuery();
      setIsGenerating(false);
      
      // Reset thinking state if active
      if (isThinkingActive) {
        setIsThinkingActive(false);
        setThinkingFromEndpoint(false);
      }
      
      // Add any partial assistant message to history
      if (assistantTextBuffer) {
        addMessage('assistant', assistantTextBuffer, latestContext);
        setAssistantTextBuffer('');
      }
    }
  };

  // Scroll chat container to bottom
  const scrollToBottom = () => {
    if (chatContainerRef.current) {
      chatContainerRef.current.scrollTop = chatContainerRef.current.scrollHeight;
    }
  };

  const value = {
    messageHistory,
    inputValue,
    setInputValue,
    isGenerating,
    handleSubmit,
    editMessage,
    stopGeneration,
    chatContainerRef,
    assistantTextBuffer,
    latestContext,
    ttftValue,
    isThinkingActive,
    thinkingContent,
    thinkingStartTime,
    scrollToBottom
  };

  return (
    <ChatContext.Provider value={value}>
      {children}
    </ChatContext.Provider>
  );
};