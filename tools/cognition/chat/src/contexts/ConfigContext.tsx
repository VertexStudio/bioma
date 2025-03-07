import React, {
  createContext,
  useState,
  useContext,
  useEffect,
  useCallback,
  ReactNode,
} from "react";
import { useLocalStorage } from "../hooks/useLocalStorage";

interface Source {
  path: string;
  active: boolean;
  isLocal: boolean;
}

interface SourcesState {
  sources: Source[];
  sourcesExpanded: Record<string, boolean>;
}

interface Tool {
  id: string;
  actor: string;
  active: boolean;
}

interface ConfigContextType {
  // Main settings
  sidebarVisible: boolean;
  toggleSidebar: () => void;
  toolsEnabled: boolean;
  toggleTools: () => void;
  thinkEnabled: boolean;
  toggleThink: () => void;

  // Sources
  sourcesDetailsOpen: boolean;
  setSourcesDetailsOpen: (state: boolean) => void;
  savedSources: SourcesState;
  setSavedSources: (sources: SourcesState) => void;
  localSources: Source[];
  activeSources: Source[];
  addNewSource: (path: string) => boolean;
  setSourceState: (sourcePath: string, state: boolean) => void;
  deleteLocalSource: (sourcePath: string) => void;
  deleteAllSources: () => void;

  // Tools
  toolsDetailsOpen: boolean;
  setToolsDetailsOpen: (state: boolean) => void;
  savedTools: Tool[];
  activeTools: Tool[];
  addNewTool: (actor: string) => boolean;
  setToolState: (toolId: string, state: boolean) => void;
  deleteTool: (toolId: string) => void;
  deleteAllTools: () => void;
}

interface ConfigProviderProps {
  children: ReactNode;
}

const ConfigContext = createContext<ConfigContextType | undefined>(undefined);

export const useConfig = (): ConfigContextType => {
  const context = useContext(ConfigContext);
  if (context === undefined) {
    throw new Error("useConfig must be used within a ConfigProvider");
  }
  return context;
};

export const ConfigProvider: React.FC<ConfigProviderProps> = ({ children }) => {
  // Main application settings
  const [sidebarVisible, setSidebarVisible] = useLocalStorage<boolean>(
    "sidebarVisible",
    true
  );
  const [toolsEnabled, setToolsEnabled] = useLocalStorage<boolean>(
    "toolsEnabled",
    false
  );
  const [thinkEnabled, setThinkEnabled] = useLocalStorage<boolean>(
    "thinkEnabled",
    false
  );

  // Sources settings
  const [sourcesDetailsOpen, setSourcesDetailsOpen] = useLocalStorage<boolean>(
    "sourcesDetailsOpen",
    true
  );
  const [savedSources, setSavedSources] = useLocalStorage<SourcesState>(
    "savedSources",
    { sources: [], sourcesExpanded: {} }
  );
  const [localSources, setLocalSources] = useLocalStorage<Source[]>(
    "localSources",
    []
  );
  const [activeSources, setActiveSources] = useState<Source[]>([]);

  // Tools settings
  const [toolsDetailsOpen, setToolsDetailsOpen] = useLocalStorage<boolean>(
    "toolsDetailsOpen",
    true
  );
  const [savedTools, setSavedTools] = useLocalStorage<Tool[]>("savedTools", []);
  const [activeTools, setActiveTools] = useState<Tool[]>([]);

  // Update active sources whenever saved sources change
  useEffect(() => {
    const newActiveSources = savedSources.sources
      ? savedSources.sources.filter((source) => source.active)
      : [];
    setActiveSources(newActiveSources);
  }, [savedSources.sources]);

  // Update active tools whenever saved tools change
  useEffect(() => {
    const newActiveTools = savedTools
      ? savedTools.filter((tool) => tool.active)
      : [];
    setActiveTools(newActiveTools);
  }, [savedTools]);

  // Separate effect for disabling tools when none are active
  useEffect(() => {
    // Disable tools if no tools are active
    if (activeTools.length === 0 && toolsEnabled) {
      setToolsEnabled(false);
    }
  }, [activeTools.length, toolsEnabled, setToolsEnabled]);

  // Update toggle functions to be more reliable
  const toggleSidebar = useCallback(() => {
    // Force the change to be visible immediately
    const newValue = !sidebarVisible;
    console.log(`[ConfigContext] Setting sidebarVisible to: ${newValue}`);

    // Use the function setter to ensure we're working with the latest state
    setSidebarVisible(newValue);

    // Also directly update localStorage as a backup in case the hook is failing
    try {
      window.localStorage.setItem("sidebarVisible", JSON.stringify(newValue));
    } catch (e) {
      console.error("Error saving to localStorage:", e);
    }
  }, [sidebarVisible, setSidebarVisible]);

  const toggleTools = useCallback(() => {
    if (activeTools.length > 0) {
      setToolsEnabled((prevEnabled) => !prevEnabled);
    }
  }, [activeTools.length, setToolsEnabled]);

  const toggleThink = useCallback(() => {
    setThinkEnabled((prevEnabled) => !prevEnabled);
  }, [setThinkEnabled]);

  // Sources functions
  const addNewSource = (path: string): boolean => {
    if (!path) return false;

    let sourcePath = path;
    if (sourcePath[0] !== "/") {
      sourcePath = `/${sourcePath}`;
    }

    // Check if source already exists
    const allSources = [...(savedSources.sources || []), ...localSources];
    if (allSources.some((source) => source.path === sourcePath)) {
      return false; // Source already exists
    }

    const newSource: Source = {
      path: sourcePath,
      active: false,
      isLocal: true,
    };

    setLocalSources([...localSources, newSource]);
    return true;
  };

  const setSourceState = (sourcePath: string, state: boolean): void => {
    setSavedSources({
      ...savedSources,
      sources: savedSources.sources.map((source) =>
        source.path === sourcePath ? { ...source, active: state } : source
      ),
    });
  };

  const deleteLocalSource = (sourcePath: string): void => {
    setLocalSources(
      localSources.filter((source) => source.path !== sourcePath)
    );
  };

  const deleteAllSources = (): void => {
    setSavedSources({ sources: [], sourcesExpanded: {} });
    setLocalSources([]);
  };

  // Tools functions
  const addNewTool = (actor: string): boolean => {
    if (!actor) return false;

    // Check if tool already exists
    if (
      savedTools.some(
        (tool) => tool.actor.toLowerCase() === actor.toLowerCase()
      )
    ) {
      return false; // Tool already exists
    }

    const newTool: Tool = {
      id: Date.now().toString(),
      actor,
      active: false,
    };

    setSavedTools([...savedTools, newTool]);
    return true;
  };

  const setToolState = (toolId: string, state: boolean): void => {
    setSavedTools(
      savedTools.map((tool) =>
        tool.id === toolId ? { ...tool, active: state } : tool
      )
    );
  };

  const deleteTool = (toolId: string): void => {
    setSavedTools(savedTools.filter((tool) => tool.id !== toolId));
  };

  const deleteAllTools = (): void => {
    setSavedTools([]);
  };

  const value: ConfigContextType = {
    // Main settings
    sidebarVisible,
    toggleSidebar,
    toolsEnabled,
    toggleTools,
    thinkEnabled,
    toggleThink,

    // Sources
    sourcesDetailsOpen,
    setSourcesDetailsOpen,
    savedSources,
    setSavedSources,
    localSources,
    activeSources,
    addNewSource,
    setSourceState,
    deleteLocalSource,
    deleteAllSources,

    // Tools
    toolsDetailsOpen,
    setToolsDetailsOpen,
    savedTools,
    activeTools,
    addNewTool,
    setToolState,
    deleteTool,
    deleteAllTools,
  };

  return (
    <ConfigContext.Provider value={value}>{children}</ConfigContext.Provider>
  );
};
