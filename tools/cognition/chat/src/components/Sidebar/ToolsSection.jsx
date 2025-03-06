import React, { useState } from 'react';
import { useConfig } from '../../contexts/ConfigContext';
import './ToolsSection.css';

const ToolsSection = () => {
  const {
    toolsDetailsOpen,
    setToolsDetailsOpen,
    savedTools,
    addNewTool,
    setToolState,
    deleteTool,
    deleteAllTools
  } = useConfig();
  
  const [toolInput, setToolInput] = useState('');
  
  const handleAddTool = () => {
    if (toolInput.trim()) {
      const success = addNewTool(toolInput);
      if (success) {
        setToolInput('');
      } else {
        alert('This tool already exists!');
      }
    }
  };
  
  const handleToolToggle = (toolId, isActive) => {
    setToolState(toolId, isActive);
  };
  
  return (
    <details
      id="tools-details"
      className="details details-transition-fix"
      open={toolsDetailsOpen}
      onToggle={(e) => setToolsDetailsOpen(e.target.open)}
    >
      <summary>
        <i className="fas fa-toolbox" style={{ marginRight: '8px' }}></i> Tools
      </summary>
      
      <div className="section tools-section">
        <details className="input-detail">
          <summary className="input-summary">
            <h4><i className="fas fa-plus-circle"></i> Add Agent</h4>
          </summary>
          
          <div className="source-content">
            <div className="tools-form" id="tool-form">
              <div className="form-group">
                <input 
                  type="text" 
                  placeholder="Agent name" 
                  value={toolInput}
                  onChange={(e) => setToolInput(e.target.value)}
                  onKeyPress={(e) => e.key === 'Enter' && handleAddTool()}
                />
              </div>
            </div>
            <div className="form-actions">
              <button 
                id="save-tool-btn" 
                className="action-button"
                onClick={handleAddTool}
              >
                <i className="fas fa-save"></i> Save
              </button>
            </div>
          </div>
        </details>
        
        <input type="hidden" id="tools-actors-input" value="" />
        
        <div className="saved-tools-section">
          <div className="available-header">
            <h3 className="section-inner-title" style={{ marginBottom: '10px' }}>
              <i className="fas fa-archive"></i> Available Tools
            </h3>
            {savedTools.length > 0 && (
              <h4
                id="delete-all-tools"
                className="delete-all"
                style={{ marginBottom: '10px' }}
                onClick={deleteAllTools}
              >
                <i className="fa-solid fa-trash"></i>
                <div style={{ marginLeft: '5px' }}>Delete all</div>
              </h4>
            )}
          </div>
          
          <div id="saved-tools-list" className="tools-list">
            {savedTools.map(tool => (
              <ToolItem
                key={tool.id}
                tool={tool}
                onToggle={(isActive) => handleToolToggle(tool.id, isActive)}
                onDelete={() => deleteTool(tool.id)}
              />
            ))}
          </div>
        </div>
      </div>
    </details>
  );
};

const ToolItem = ({ tool, onToggle, onDelete }) => {
  return (
    <div 
      className={`tool-item ${tool.active ? 'active' : ''}`}
      data-tooltip={tool.actor}
      data-tool-id={tool.id}
    >
      <div className="tool-item-header">
        <span className="tool-icon"><i className="fas fa-wrench"></i></span>
        <span className="tool-item-name">{tool.actor}</span>
        <label className="switch">
          <input 
            type="checkbox" 
            className="tool-toggle" 
            checked={tool.active}
            onChange={(e) => onToggle(e.target.checked)}
          />
          <span className="slider"></span>
        </label>
        <div className="tool-actions">
          <button 
            className="tool-action-btn delete-tool" 
            title="Delete"
            onClick={(e) => {
              e.stopPropagation();
              onDelete();
            }}
          >
            <i className="fas fa-trash"></i>
          </button>
        </div>
      </div>
    </div>
  );
};

export default ToolsSection;