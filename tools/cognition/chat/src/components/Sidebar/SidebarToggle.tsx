import React, { useEffect, useState } from "react";
import { useConfig } from "../../contexts/ConfigContext";
import "./SidebarToggle.css";

const SidebarToggle: React.FC = () => {
  const { toggleSidebar, sidebarVisible } = useConfig();
  const [internalVisible, setInternalVisible] =
    useState<boolean>(sidebarVisible);

  // Keep internal state in sync with context
  useEffect(() => {
    setInternalVisible(sidebarVisible);
  }, [sidebarVisible]);

  const handleToggle = (): void => {
    console.log(
      `[SidebarToggle] Toggle clicked. Current state: ${
        sidebarVisible ? "visible" : "collapsed"
      }`
    );

    // Update internal state immediately for responsiveness
    setInternalVisible(!sidebarVisible);

    // Call the actual toggle function from context
    toggleSidebar();
  };

  return (
    <button
      id="sidebar-toggle"
      onClick={handleToggle}
      aria-label={sidebarVisible ? "Collapse sidebar" : "Expand sidebar"}
      data-visible={sidebarVisible.toString()}
    >
      <svg
        xmlns="http://www.w3.org/2000/svg"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        style={{
          transform: sidebarVisible ? "rotate(180deg)" : "rotate(0deg)",
        }}
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
