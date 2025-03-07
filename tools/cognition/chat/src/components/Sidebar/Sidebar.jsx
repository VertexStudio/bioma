import React, { useEffect, useRef, useState } from 'react';
import { useConfig } from '../../contexts/ConfigContext';
import { useResizableElement } from '../../hooks/useResizableElement';
import SidebarToggle from './SidebarToggle';
import SourcesSection from './SourcesSection';
import ToolsSection from './ToolsSection';
import './Sidebar.css';

const Sidebar = () => {
  const { sidebarVisible } = useConfig();
  // Add local state to force re-renders
  const [isVisible, setIsVisible] = useState(sidebarVisible);
  
  const { 
    elementRef, 
    resizerRef, 
    currentWidthRef,
    lastWidthRef,
    saveCurrentWidth,
    restoreWidth
  } = useResizableElement({
    minWidth: 280,
    maxWidth: 550,
    defaultWidth: 350
  });
  
  const prevVisibleRef = useRef(sidebarVisible);
  const initializedRef = useRef(false);

  // Add the details-loaded class once on mount
  useEffect(() => {
    // Add details-loaded class to all details elements after mount
    const detailsElements = document.querySelectorAll('.details-transition-fix');
    detailsElements.forEach(details => {
      details.classList.add('details-loaded');
    });
    
    initializedRef.current = true;
    setIsVisible(sidebarVisible);
  }, [sidebarVisible]);
  
  // Handle sidebar visibility changes without causing re-renders
  useEffect(() => {
    // Skip the first render to avoid conflicts with initialization
    if (!initializedRef.current) return;
    
    if (sidebarVisible !== prevVisibleRef.current) {
      console.log(`[Sidebar] Visibility changed: ${sidebarVisible ? 'visible' : 'collapsed'}`);
      console.log(`[Sidebar] Current width: ${currentWidthRef.current}px, Last saved width: ${lastWidthRef.current}px`);
      
      // Update local state to force a re-render
      setIsVisible(sidebarVisible);
      
      if (sidebarVisible && !prevVisibleRef.current) {
        // Expanding sidebar, restore previous width
        console.log(`[Sidebar] Restoring width to ${lastWidthRef.current}px`);
        restoreWidth();
      } else if (!sidebarVisible && prevVisibleRef.current) {
        // Collapsing sidebar, save current width
        const width = saveCurrentWidth();
        console.log(`[Sidebar] Saving width before collapse: ${width}px`);
      }
      
      prevVisibleRef.current = sidebarVisible;
    }
  }, [sidebarVisible, restoreWidth, saveCurrentWidth]);
  
  // Force the sidebar to match the current state
  useEffect(() => {
    if (elementRef.current) {
      if (sidebarVisible) {
        elementRef.current.classList.remove('collapsed');
      } else {
        elementRef.current.classList.add('collapsed');
      }
    }
  }, [sidebarVisible, elementRef]);
  
  // Get class based on both the context value and local state
  const sidebarClass = sidebarVisible ? '' : 'collapsed';
  
  return (
    <div 
      id="sidebar" 
      ref={elementRef}
      className={sidebarClass}
      data-visible={sidebarVisible.toString()}
    >
      <div className="sidebar-header">
        <h2>
          <i className="fa-solid fa-gear" style={{ marginRight: '8px' }}></i> Config
        </h2>
        <SidebarToggle />
      </div>
      
      <SourcesSection />
      <ToolsSection />
      
      <div id="resizer" ref={resizerRef} className="resizer"></div>
    </div>
  );
};

export default Sidebar;