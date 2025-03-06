import React, { useState, useEffect, useRef } from 'react';
import { useConfig } from '../../contexts/ConfigContext';
import { useRAGService } from '../../hooks/useRAGService';
import './SourcesSection.css';

const SourcesSection = () => {
  const {
    sourcesDetailsOpen,
    setSourcesDetailsOpen,
    savedSources,
    setSavedSources,
    localSources,
    addNewSource,
    setSourceState,
    deleteLocalSource,
    deleteAllSources
  } = useConfig();
  
  const { fetchAvailableSources, uploadAndIndexSource, deleteSource } = useRAGService({});
  
  const [sourceInput, setSourceInput] = useState('');
  const [isLoading, setIsLoading] = useState(false);
  const fileInputRef = useRef(null);
  
  // Fetch available sources when component mounts or section is opened
  useEffect(() => {
    if (sourcesDetailsOpen) {
      loadSources();
    }
  }, [sourcesDetailsOpen]);
  
  const loadSources = async () => {
    setIsLoading(true);
    try {
      const data = await fetchAvailableSources();
      
      // Process the sources data
      const mappedSources = data.sources.map(source => ({
        path: source.source,
        active: false,
        uri: source.uri
      }));
      
      // Group sources by path
      const grouped = {};
      mappedSources.forEach(source => {
        if (!grouped[source.path]) {
          grouped[source.path] = [];
        }
        grouped[source.path].push(source);
      });
      
      // Update saved sources with any existing activation state
      const existingSources = savedSources.sources || [];
      const mergedSources = Object.keys(grouped).map(path => {
        const existing = existingSources.find(s => s.path === path);
        return existing || { path, active: false };
      });
      
      setSavedSources({
        sources: mergedSources,
        sourcesExpanded: grouped
      });
    } catch (error) {
      console.error('Error fetching sources:', error);
    } finally {
      setIsLoading(false);
    }
  };
  
  const handleAddSource = () => {
    if (sourceInput.trim()) {
      const success = addNewSource(sourceInput);
      if (success) {
        setSourceInput('');
      } else {
        alert('Source already exists.');
      }
    }
  };
  
  const handleSourceToggle = (sourcePath, isActive) => {
    setSourceState(sourcePath, isActive);
  };
  
  const handleFileUpload = async (file, sourcePath) => {
    if (!file) return;
    
    try {
      await uploadAndIndexSource(file, sourcePath);
      // Refresh sources after upload
      await loadSources();
    } catch (error) {
      console.error('Error uploading file:', error);
      alert(`Upload failed: ${error.message}`);
    }
  };
  
  const handleDeleteSource = async (source) => {
    try {
      if (source.isLocal) {
        deleteLocalSource(source.path);
      } else {
        await deleteSource([source.path]);
        await loadSources();
      }
    } catch (error) {
      console.error('Error deleting source:', error);
      alert(`Delete failed: ${error.message}`);
    }
  };
  
  const handleDeleteAllSources = async () => {
    try {
      const allPaths = savedSources.sources.map(source => source.path);
      if (allPaths.length > 0) {
        await deleteSource(allPaths);
      }
      deleteAllSources();
      await loadSources();
    } catch (error) {
      console.error('Error deleting all sources:', error);
      alert(`Delete failed: ${error.message}`);
    }
  };
  
  // Combine server sources and local sources for display
  const allSources = [
    ...(savedSources.sources || []),
    ...localSources
  ];
  
  return (
    <details 
      id="sources-details" 
      className="details details-transition-fix"
      open={sourcesDetailsOpen}
      onToggle={(e) => setSourcesDetailsOpen(e.target.open)}
    >
      <summary>
        <i className="fa-solid fa-book" style={{ marginRight: '8px' }}></i>
        Sources
      </summary>
      
      <div className="section source-section">
        <details className="input-detail">
          <summary className="input-summary">
            <h4><i className="fas fa-plus-circle"></i> Add Source</h4>
          </summary>
          <div className="source-content">
            <div className="source-form" id="source-form">
              <div className="form-group">
                <input 
                  type="text" 
                  placeholder="/global" 
                  value={sourceInput}
                  onChange={(e) => setSourceInput(e.target.value)}
                  onKeyPress={(e) => e.key === 'Enter' && handleAddSource()}
                />
              </div>
            </div>
            
            <button
              className="new-source action-button"
              onClick={handleAddSource}
            >
              Add new source
            </button>
          </div>
        </details>
        
        <div id="active-sources-list" className="sources-list">
          {allSources.filter(source => source.active).map(source => (
            <SourceItem 
              key={source.path} 
              source={source}
              sourcesExpanded={savedSources.sourcesExpanded}
              onToggle={(isActive) => handleSourceToggle(source.path, isActive)}
              onDelete={() => handleDeleteSource(source)}
              onFileUpload={handleFileUpload}
            />
          ))}
        </div>
        
        <div className="saved-sources-section">
          <div className="available-header">
            <h3 className="section-inner-title" style={{ marginBottom: '10px' }}>
              <i className="fas fa-archive"></i> Available Sources
            </h3>
            {allSources.length > 0 && (
              <h4
                className="delete-all"
                style={{ marginBottom: '10px' }}
                onClick={handleDeleteAllSources}
              >
                <i className="fa-solid fa-trash"></i>
                <div style={{ marginLeft: '5px' }}>
                  {isLoading ? 'Deleting...' : 'Delete all'}
                </div>
              </h4>
            )}
          </div>
          
          <div id="saved-sources-list" className="sources-list">
            {isLoading ? (
              <div className="loading-indicator">
                <span>Loading sources...</span>
              </div>
            ) : (
              allSources
                .filter(source => !source.active)
                .map(source => (
                  <SourceItem 
                    key={source.path} 
                    source={source}
                    sourcesExpanded={savedSources.sourcesExpanded}
                    onToggle={(isActive) => handleSourceToggle(source.path, isActive)}
                    onDelete={() => handleDeleteSource(source)}
                    onFileUpload={handleFileUpload}
                  />
                ))
            )}
          </div>
        </div>
      </div>
    </details>
  );
};

const SourceItem = ({ source, sourcesExpanded, onToggle, onDelete, onFileUpload }) => {
  const [expanded, setExpanded] = useState(false);
  const [isUploading, setIsUploading] = useState(false);
  const fileInputRef = useRef(null);
  
  const handleFileSelect = async (event) => {
    const file = event.target.files[0];
    if (!file) return;
    
    setIsUploading(true);
    try {
      await onFileUpload(file, source.path);
    } finally {
      setIsUploading(false);
      // Clear the file input
      if (fileInputRef.current) {
        fileInputRef.current.value = '';
      }
    }
  };
  
  // Get source items for this path from expanded sources
  const sourceItems = !source.isLocal && sourcesExpanded && sourcesExpanded[source.path] 
    ? sourcesExpanded[source.path] 
    : [];
  
  return (
    <div 
      className={`source-item ${source.active ? 'active' : ''}`}
      data-tooltip={source.path}
    >
      <div className="source-item-header">
        <span className="source-icon"><i className="fa-solid fa-book"></i></span>
        <span className="source-item-name">{source.path}</span>
        <span className="number-of-elements">({sourceItems.length} items)</span>
        
        {!source.isLocal && (
          <label className="switch">
            <input 
              type="checkbox" 
              className="source-toggle" 
              checked={source.active} 
              onChange={(e) => onToggle(e.target.checked)}
            />
            <span className="slider"></span>
          </label>
        )}
        
        <span 
          className="upload-icon upload-button" 
          title="Upload file to source"
          onClick={() => fileInputRef.current?.click()}
        >
          <i className="fa-solid fa-paperclip"></i>
          <input
            ref={fileInputRef}
            type="file"
            className="file-input"
            style={{ display: 'none' }}
            onChange={handleFileSelect}
          />
        </span>
        
        {isUploading && (
          <span className="upload-status">Uploading...</span>
        )}
        
        <div className="source-actions">
          <button 
            className="source-action-btn delete-source" 
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
      
      {sourceItems.length > 0 && (
        <div 
          className="source-item-list" 
          style={{ display: expanded ? 'block' : 'none' }}
          onClick={(e) => e.stopPropagation()}
        >
          <ul>
            {sourceItems.map((item, index) => (
              <li key={index}>{item.uri}</li>
            ))}
          </ul>
        </div>
      )}
      
      {sourceItems.length > 0 && (
        <div 
          className="source-item-expander"
          onClick={() => setExpanded(!expanded)}
        >
          {expanded ? 'Hide items' : 'Show items'} ▾
        </div>
      )}
    </div>
  );
};

export default SourcesSection;