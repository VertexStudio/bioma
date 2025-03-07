import React, { useEffect } from "react";
import { useChat } from "../../contexts/ChatContext";
import { useConfig } from "../../contexts/ConfigContext";
import UserMessage from "./UserMessage.tsx";
import AssistantMessage from "./AssistantMessage.tsx";
import SystemMessage from "./SystemMessage.tsx";
import ToolMessage from "./ToolMessage.tsx";
import ErrorMessage from "./ErrorMessage.tsx";
import ThinkingProcess from "./ThinkingProcess.tsx";
import "./ChatContainer.css";

interface Message {
  role: "user" | "assistant" | "system" | "tool" | "error";
  content: string;
  context?: any;
}

const ChatContainer: React.FC = () => {
  const {
    messageHistory,
    chatContainerRef,
    isThinkingActive,
    thinkingContent,
    thinkingStartTime,
    scrollToBottom,
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
      className={sidebarVisible ? "" : "sidebar-collapsed"}
    >
      {messageHistory.map((message: Message, index: number) => {
        switch (message.role) {
          case "user":
            return (
              <UserMessage
                key={`user-${index}`}
                content={message.content}
                index={index}
              />
            );
          case "assistant":
            return (
              <AssistantMessage
                key={`assistant-${index}`}
                content={message.content}
                context={message.context}
                index={index}
              />
            );
          case "system":
            return (
              <SystemMessage
                key={`system-${index}`}
                content={message.content}
              />
            );
          case "tool":
            return (
              <ToolMessage key={`tool-${index}`} content={message.content} />
            );
          case "error":
            return (
              <ErrorMessage key={`error-${index}`} error={message.content} />
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
