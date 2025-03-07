import React, { createContext, useState, useContext, useEffect, useCallback } from 'react';
import { useLocalStorage } from '../hooks/useLocalStorage';

const ConfigContext = createContext();

export const useConfig = () => useContext(ConfigContext);

export const ConfigProvider = ({ children }) => {
  // Main application settings
  const [sidebarVisible, setSidebarVisible] = useLocalStorage('sidebarVisible', true);
  const [toolsEnabled, setToolsEnabled] = useLocalStorage('toolsEnabled', false);
  const [thinkEnabled, setThinkEnabled] = useLocalStorage('thinkEnabled', false);
  
  // Sources settings
  const [sourcesDetailsOpen, setSourcesDetailsOpen] = useLocalStorage('sourcesDetailsOpen', true);
  const [savedSources, setSavedSources] = useLocalStorage('savedSources', { sources: [], sourcesExpanded: {} });
  const [localSources, setLocalSources] = useLocalStorage('localSources', []);
  const [activeSources, setActiveSources] = useState([]);
  
  // Tools settings
  const [toolsDetailsOpen, setToolsDetailsOpen] = useLocalStorage('toolsDetailsOpen', true);
  const [savedTools, setSavedTools] = useLocalStorage('savedTools', []);
  const [activeTools, setActiveTools] = useState([]);

  // Update active sources whenever saved sources change
  useEffect(() => {
    const newActiveSources = savedSources.sources ? 
      savedSources.sources.filter(source => source.active) : [];
    setActiveSources(newActiveSources);
  }, [savedSources.sources]);

  // Update active tools whenever saved tools change
  useEffect(() => {
    const newActiveTools = savedTools ? 
      savedTools.filter(tool => tool.active) : [];
    setActiveTools(newActiveTools);
  }, [savedTools]);
  
  // Separate effect for disabling tools when none are active
  useEffect(() => {
    // Disable tools if no tools are active
    if (activeTools.length === 0 && toolsEnabled) {
      setToolsEnabled(false);
    }
  }, [activeTools.length, toolsEnabled]);

  // Update toggle functions to be more reliable
  const toggleSidebar = useCallback(() => {
    // Force the change to be visible immediately
    const newValue = !sidebarVisible;
    console.log(`[ConfigContext] Setting sidebarVisible to: ${newValue}`);
    
    // Use the function setter to ensure we're working with the latest state
    setSidebarVisible(newValue);
    
    // Also directly update localStorage as a backup in case the hook is failing
    try {
      window.localStorage.setItem('sidebarVisible', JSON.stringify(newValue));
    } catch (e) {
      console.error('Error saving to localStorage:', e);
    }
  }, [sidebarVisible, setSidebarVisible]);

  const toggleTools = useCallback(() => {
    if (activeTools.length > 0) {
      setToolsEnabled(prevEnabled => !prevEnabled);
    }
  }, [activeTools.length]);

  const toggleThink = useCallback(() => {
    setThinkEnabled(prevEnabled => !prevEnabled);
  }, []);

  // Sources functions
  const addNewSource = (path) => {
    if (!path) return;
    
    let sourcePath = path;
    if (sourcePath[0] !== '/') {
      sourcePath = `/${sourcePath}`;
    }

    // Check if source already exists
    const allSources = [...(savedSources.sources || []), ...localSources];
    if (allSources.some(source => source.path === sourcePath)) {
      return false; // Source already exists
    }

    const newSource = {
      path: sourcePath,
      active: false,
      isLocal: true,
    };

    setLocalSources([...localSources, newSource]);
    return true;
  };

  const setSourceState = (sourcePath, state) => {
    setSavedSources({
      ...savedSources,
      sources: savedSources.sources.map(source => 
        source.path === sourcePath ? { ...source, active: state } : source
      )
    });
  };

  const deleteLocalSource = (sourcePath) => {
    setLocalSources(localSources.filter(source => source.path !== sourcePath));
  };

  const deleteAllSources = () => {
    setSavedSources({ sources: [], sourcesExpanded: {} });
    setLocalSources([]);
  };

  // Tools functions
  const addNewTool = (actor) => {
    if (!actor) return;

    // Check if tool already exists
    if (savedTools.some(tool => tool.actor.toLowerCase() === actor.toLowerCase())) {
      return false; // Tool already exists
    }

    const newTool = {
      id: Date.now().toString(),
      actor,
      active: false,
    };

    setSavedTools([...savedTools, newTool]);
    return true;
  };

  const setToolState = (toolId, state) => {
    setSavedTools(savedTools.map(tool => 
      tool.id === toolId ? { ...tool, active: state } : tool
    ));
  };

  const deleteTool = (toolId) => {
    setSavedTools(savedTools.filter(tool => tool.id !== toolId));
  };

  const deleteAllTools = () => {
    setSavedTools([]);
  };

  const value = {
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
    <ConfigContext.Provider value={value}>
      {children}
    </ConfigContext.Provider>
  );
};