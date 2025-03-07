import React, {
  createContext,
  useState,
  useContext,
  useRef,
  ReactNode,
} from "react";
import { useConfig } from "./ConfigContext";
import { useRAGService } from "../hooks/useRAGService.ts";

interface Message {
  role: "user" | "assistant" | "system";
  content: string;
}

interface ChatContextType {
  messageHistory: Message[];
  inputValue: string;
  setInputValue: (value: string) => void;
  isGenerating: boolean;
  assistantTextBuffer: string;
  latestContext: any | null;
  contextSet: boolean;
  ttftValue: number | null;
  isThinkingActive: boolean;
  thinkingContent: string;
  thinkingStartTime: Date | null;
  thinkingFromEndpoint: boolean;
  chatContainerRef: React.RefObject<HTMLDivElement | null>;
  handleSubmit: (override?: string | null) => Promise<void>;
  stopGeneration: () => void;
  addMessage: (role: Message["role"], content: string, context?: any) => void;
  editMessage: (index: number, newContent: string) => void;
  scrollToBottom: () => void;
}

interface ChatProviderProps {
  children: ReactNode;
}

const ChatContext = createContext<ChatContextType | undefined>(undefined);

export const useChat = (): ChatContextType => {
  const context = useContext(ChatContext);
  if (context === undefined) {
    throw new Error("useChat must be used within a ChatProvider");
  }
  return context;
};

export const ChatProvider: React.FC<ChatProviderProps> = ({ children }) => {
  const { toolsEnabled, thinkEnabled, activeTools, activeSources } =
    useConfig();

  // Chat state
  const [messageHistory, setMessageHistory] = useState<Message[]>([]);
  const [inputValue, setInputValue] = useState("");
  const [isGenerating, setIsGenerating] = useState(false);
  const [assistantTextBuffer, setAssistantTextBuffer] = useState("");
  const [latestContext, setLatestContext] = useState<any | null>(null);
  const [contextSet, setContextSet] = useState(false);
  const [ttftValue, setTtftValue] = useState<number | null>(null);

  // Thinking process state
  const [isThinkingActive, setIsThinkingActive] = useState(false);
  const [thinkingContent, setThinkingContent] = useState("");
  const [thinkingStartTime, setThinkingStartTime] = useState<Date | null>(null);
  const [thinkingFromEndpoint, setThinkingFromEndpoint] = useState(false);

  // References
  const chatContainerRef = useRef<HTMLDivElement | null>(null);
  const controllerRef = useRef<AbortController | null>(null);

  // Add a new message to the chat
  const addMessage = (
    role: Message["role"],
    content: string,
    context: any = null
  ): void => {
    const newMessage: Message = { role, content };
    setMessageHistory((prev) => [...prev, newMessage]);

    if (role === "assistant" && context) {
      setLatestContext(context);
    }
  };

  // Functions to handle RAG service responses
  function handleResponse(data: any): void {
    // Handle streaming response
    if (data.text) {
      setAssistantTextBuffer((prev) => prev + data.text);
    }

    // Handle thinking
    if (data.thinking && thinkEnabled) {
      if (!isThinkingActive) {
        setIsThinkingActive(true);
        setThinkingStartTime(new Date());
        setThinkingFromEndpoint(true);
      }
      setThinkingContent(data.thinking);
    }

    // Handle TTFT (Time to First Token)
    if (!ttftValue && data.text && data.text.trim().length > 0) {
      const now = new Date();
      const startTime = thinkingStartTime || now;
      const ttft = now.getTime() - startTime.getTime();
      setTtftValue(ttft);
    }
  }

  function handleError(error: Error | string): void {
    console.error("Error in chat request:", error);
    setIsGenerating(false);
    setIsThinkingActive(false);

    // Add error message to chat
    addMessage(
      "assistant",
      "Sorry, there was an error processing your request. Please try again."
    );
  }

  function handleComplete(): void {
    if (assistantTextBuffer) {
      addMessage("assistant", assistantTextBuffer);
      setAssistantTextBuffer("");
    }

    setIsGenerating(false);
    setIsThinkingActive(false);
    setThinkingContent("");
    setThinkingStartTime(null);
    setThinkingFromEndpoint(false);
    setTtftValue(null);

    controllerRef.current = null;
  }

  const { sendQuery, abortQuery } = useRAGService({
    onResponse: handleResponse,
    onError: handleError,
    onComplete: handleComplete,
  });

  // Handle chat submission
  const handleSubmit = async (
    override: string | null = null
  ): Promise<void> => {
    if (isGenerating && !override) return;

    const userInput = override || inputValue;
    if (!userInput.trim()) return;

    // Add user message to history
    addMessage("user", userInput);

    // Reset state for new generation
    setInputValue("");
    setIsGenerating(true);
    setAssistantTextBuffer("");
    setContextSet(false);

    // Initialize thinking state if enabled
    if (thinkEnabled) {
      setIsThinkingActive(true);
      setThinkingContent("");
      setThinkingStartTime(new Date());
      setThinkingFromEndpoint(false);
    }

    try {
      // Create an abort controller for this request
      controllerRef.current = new AbortController();

      // Prepare the query parameters
      const queryParams = {
        message: userInput,
        tools: toolsEnabled && activeTools.length > 0 ? activeTools : [],
        sources: activeSources.length > 0 ? activeSources : [],
        think: thinkEnabled,
        signal: controllerRef.current.signal,
      };

      // Send the query to the RAG service
      await sendQuery(queryParams);
    } catch (error) {
      console.error("Failed to send chat request:", error);
      handleError(error as Error);
    }
  };

  // Edit a message in the chat history
  const editMessage = (index: number, newContent: string): void => {
    setMessageHistory((prev) => {
      const newHistory = [...prev];
      if (index >= 0 && index < newHistory.length) {
        newHistory[index] = { ...newHistory[index], content: newContent };
      }
      return newHistory;
    });
  };

  // Stop the ongoing generation
  const stopGeneration = (): void => {
    if (controllerRef.current) {
      abortQuery();
      controllerRef.current = null;
    }

    // Finalize assistant response if any
    if (assistantTextBuffer) {
      addMessage("assistant", assistantTextBuffer);
      setAssistantTextBuffer("");
    }

    setIsGenerating(false);
    setIsThinkingActive(false);
    setThinkingContent("");
    setThinkingStartTime(null);
  };

  // Scroll chat container to bottom
  const scrollToBottom = (): void => {
    if (chatContainerRef.current) {
      chatContainerRef.current.scrollTop =
        chatContainerRef.current.scrollHeight;
    }
  };

  // Create context value
  const contextValue: ChatContextType = {
    messageHistory,
    inputValue,
    setInputValue,
    isGenerating,
    assistantTextBuffer,
    latestContext,
    contextSet,
    ttftValue,
    isThinkingActive,
    thinkingContent,
    thinkingStartTime,
    thinkingFromEndpoint,
    chatContainerRef,
    handleSubmit,
    stopGeneration,
    addMessage,
    editMessage,
    scrollToBottom,
  };

  return (
    <ChatContext.Provider value={contextValue}>{children}</ChatContext.Provider>
  );
};
