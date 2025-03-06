import React from 'react';
import { useConfig } from '../../contexts/ConfigContext';
import { useResizableElement } from '../../hooks/useResizableElement';
import SidebarToggle from './SidebarToggle';
import SourcesSection from './SourcesSection';
import ToolsSection from './ToolsSection';
import './Sidebar.css';

const Sidebar = () => {
  const { sidebarVisible } = useConfig();
  const { elementRef, resizerRef } = useResizableElement({
    minWidth: 280,
    maxWidth: 550,
    defaultWidth: 350
  });
  
  return (
    <div 
      id="sidebar" 
      ref={elementRef}
      className={sidebarVisible ? '' : 'collapsed'}
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