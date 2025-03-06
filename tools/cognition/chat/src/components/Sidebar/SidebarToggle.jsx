import React from 'react';
import { useConfig } from '../../contexts/ConfigContext';
import './SidebarToggle.css';

const SidebarToggle = () => {
  const { toggleSidebar, sidebarVisible } = useConfig();
  
  return (
    <button 
      id="sidebar-toggle" 
      onClick={toggleSidebar}
      aria-label={sidebarVisible ? 'Collapse sidebar' : 'Expand sidebar'}
    >
      <svg
        xmlns="http://www.w3.org/2000/svg"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        style={{ transform: sidebarVisible ? 'rotate(180deg)' : 'rotate(0deg)' }}
      >
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth="2"
          d="M15 19l-7-7 7-7"
        />
      </svg>
    </button>
  );
};

export default SidebarToggle;