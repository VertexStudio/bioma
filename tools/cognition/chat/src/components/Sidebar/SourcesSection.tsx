import React, {
  useState,
  useEffect,
  useRef,
  KeyboardEvent,
  MouseEvent,
  ChangeEvent,
} from "react";
import { useConfig } from "../../contexts/ConfigContext";
import { useRAGService } from "../../hooks/useRAGService.ts";
import "./SourcesSection.css";

// Use the same Source interface as in ConfigContext
interface Source {
  path: string;
  active: boolean;
  isLocal: boolean;
  uri?: string;
}

// API source item structure
interface SourceItem {
  source: string; // This is equivalent to path in our Source interface
  uri: string;
}

// API response for sources
interface SourcesResponse {
  sources: SourceItem[];
}

// Config state should match the ConfigContext definition
interface SourcesState {
  sources: Source[];
  sourcesExpanded: Record<string, boolean>;
}

// Props for the SourceItem component
interface SourceItemProps {
  source: Source;
  // sourcesExpanded expects a Record<string, boolean> from ConfigContext,
  // but we'll use a separate sourceItems prop for displaying source item details
  sourcesExpanded?: Record<string, boolean>;
  sourceItems?: SourceItem[];
  onToggle: (isActive: boolean) => void;
  onDelete: () => void;
  onFileUpload: (file: File, path: string) => Promise<void>;
}

const SourcesSection: React.FC = () => {
  const {
    sourcesDetailsOpen,
    setSourcesDetailsOpen,
    savedSources,
    setSavedSources,
    localSources,
    addNewSource,
    setSourceState,
    deleteLocalSource,
    deleteAllSources,
  } = useConfig();

  const { fetchAvailableSources, uploadAndIndexSource, deleteSource } =
    useRAGService({});

  const [sourceInput, setSourceInput] = useState<string>("");
  const [isLoading, setIsLoading] = useState<boolean>(false);
  const [sourceItemsMap, setSourceItemsMap] = useState<
    Record<string, SourceItem[]>
  >({});

  // Fetch available sources when component mounts or section is opened
  useEffect(() => {
    if (sourcesDetailsOpen) {
      loadSources();
    }
  }, [sourcesDetailsOpen]);

  const loadSources = async (): Promise<void> => {
    setIsLoading(true);
    try {
      const data = (await fetchAvailableSources()) as SourcesResponse;

      // Check if we got a valid response
      if (!data || !data.sources) {
        console.warn("Invalid response format when fetching sources:", data);
        setIsLoading(false);
        return;
      }

      // Group sources by path
      const grouped: Record<string, SourceItem[]> = {};
      data.sources.forEach((sourceItem) => {
        const path = sourceItem.source;
        if (!grouped[path]) {
          grouped[path] = [];
        }
        grouped[path].push(sourceItem);
      });

      // Store the sourceItems for display
      setSourceItemsMap(grouped);

      // Using Map for deduplication by source path
      const sourceMap = new Map<string, Source>();

      // Add existing sources to map
      const existingSources = savedSources.sources || [];
      existingSources.forEach((source) => {
        sourceMap.set(source.path, source);
      });

      // Add server sources to map (if not already present)
      Object.keys(grouped).forEach((path) => {
        if (!sourceMap.has(path)) {
          sourceMap.set(path, { path, active: false, isLocal: false });
        }
      });

      // Add local sources to map (if not already present)
      localSources.forEach((localSource) => {
        if (!sourceMap.has(localSource.path)) {
          sourceMap.set(localSource.path, localSource);
        }
      });

      // Convert back to array
      const mergedSources = Array.from(sourceMap.values());

      // Create the expanded map as a boolean record
      const expandedMap: Record<string, boolean> = {};
      Object.keys(grouped).forEach((path) => {
        // Preserve existing expanded state or default to false
        expandedMap[path] =
          savedSources.sourcesExpanded && path in savedSources.sourcesExpanded
            ? savedSources.sourcesExpanded[path]
            : false;
      });

      // Update saved sources
      setSavedSources({
        sources: mergedSources,
        sourcesExpanded: expandedMap,
      });
    } catch (error) {
      console.error("Error fetching sources:", error);
      // Don't crash the UI when sources can't be loaded
      setSavedSources({
        sources: [],
        sourcesExpanded: {},
      });
    } finally {
      setIsLoading(false);
    }
  };

  const handleAddSource = (): void => {
    if (sourceInput.trim()) {
      const success = addNewSource(sourceInput);
      if (success) {
        setSourceInput("");
      } else {
        alert("Source already exists.");
      }
    }
  };

  const handleSourceToggle = (sourcePath: string, isActive: boolean): void => {
    setSourceState(sourcePath, isActive);
  };

  const handleFileUpload = async (
    file: File,
    sourcePath: string
  ): Promise<void> => {
    if (!file) return;

    try {
      await uploadAndIndexSource(file, sourcePath);
      // Refresh sources after upload
      await loadSources();
    } catch (error) {
      console.error("Error uploading file:", error);
      alert(`Upload failed: ${(error as Error).message}`);
    }
  };

  const handleDeleteSource = async (source: Source): Promise<void> => {
    try {
      if (source.isLocal) {
        deleteLocalSource(source.path);
      } else {
        await deleteSource([source.path]);
        await loadSources();
      }
    } catch (error) {
      console.error("Error deleting source:", error);
      alert(`Delete failed: ${(error as Error).message}`);
    }
  };

  const handleDeleteAllSources = async (): Promise<void> => {
    try {
      const allPaths = savedSources.sources.map((source) => source.path);
      if (allPaths.length > 0) {
        await deleteSource(allPaths);
      }
      deleteAllSources();
      await loadSources();
    } catch (error) {
      console.error("Error deleting all sources:", error);
      alert(`Delete failed: ${(error as Error).message}`);
    }
  };

  // Combine server sources and local sources for display, ensuring no duplicates
  const allSources = React.useMemo(() => {
    // Start with saved sources
    const sources = [...(savedSources.sources || [])];

    // Add local sources that don't exist in the saved sources
    localSources.forEach((localSource) => {
      if (!sources.some((s) => s.path === localSource.path)) {
        sources.push(localSource);
      }
    });

    // Return the deduplicated list
    return sources;
  }, [savedSources.sources, localSources]);

  return (
    <details
      id="sources-details"
      className="details details-transition-fix"
      open={sourcesDetailsOpen}
      onToggle={(e) =>
        setSourcesDetailsOpen((e.target as HTMLDetailsElement).open)
      }
    >
      <summary>
        <i className="fa-solid fa-book" style={{ marginRight: "8px" }}></i>
        Sources
      </summary>

      <div className="section source-section">
        <details className="input-detail">
          <summary className="input-summary">
            <h4>
              <i className="fas fa-plus-circle"></i> Add Source
            </h4>
          </summary>
          <div className="source-content">
            <div className="source-form" id="source-form">
              <div className="form-group">
                <input
                  type="text"
                  placeholder="/global"
                  value={sourceInput}
                  onChange={(e: ChangeEvent<HTMLInputElement>) =>
                    setSourceInput(e.target.value)
                  }
                  onKeyPress={(e: KeyboardEvent<HTMLInputElement>) =>
                    e.key === "Enter" && handleAddSource()
                  }
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

        <div className="saved-sources-section">
          <div className="available-header">
            <h3
              className="section-inner-title"
              style={{ marginBottom: "10px" }}
            >
              Available Sources
            </h3>
            {allSources.length > 0 && (
              <button
                className="delete-all-btn"
                onClick={handleDeleteAllSources}
                title="Delete all sources"
              >
                Delete all
              </button>
            )}
          </div>

          <div id="saved-sources-list" className="sources-list">
            {isLoading ? (
              <div className="loading-indicator">
                <span>Loading sources...</span>
              </div>
            ) : (
              allSources.map((source) => (
                <SourceItem
                  key={source.path}
                  source={source}
                  sourcesExpanded={savedSources.sourcesExpanded}
                  sourceItems={sourceItemsMap[source.path] || []}
                  onToggle={(isActive) =>
                    handleSourceToggle(source.path, isActive)
                  }
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

const SourceItem: React.FC<SourceItemProps> = ({
  source,
  sourcesExpanded,
  sourceItems = [],
  onToggle,
  onDelete,
  onFileUpload,
}) => {
  const [expanded, setExpanded] = useState<boolean>(false);
  const [isUploading, setIsUploading] = useState<boolean>(false);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const handleFileSelect = async (
    event: ChangeEvent<HTMLInputElement>
  ): Promise<void> => {
    const file = event.target.files?.[0];
    if (!file) return;

    setIsUploading(true);
    try {
      await onFileUpload(file, source.path);
    } catch (error) {
      console.error("Error handling file:", error);
    } finally {
      setIsUploading(false);
      // Clear the input
      if (fileInputRef.current) {
        fileInputRef.current.value = "";
      }
    }
  };

  const handleAttachClick = (e: MouseEvent<HTMLSpanElement>): void => {
    e.stopPropagation();
    e.preventDefault();
    if (fileInputRef.current) {
      fileInputRef.current.click();
    }
  };

  const handleDeleteClick = (e: MouseEvent<HTMLButtonElement>): void => {
    e.stopPropagation();
    e.preventDefault();
    onDelete();
  };

  const handleSourceClick = (e: MouseEvent<HTMLDivElement>): void => {
    // Don't toggle if clicking on any interactive controls
    if (
      (e.target as HTMLElement).classList.contains("source-toggle") ||
      (e.target as HTMLElement).closest(".switch") ||
      (e.target as HTMLElement).closest(".source-actions") ||
      (e.target as HTMLElement).closest(".upload-button") ||
      (e.target as HTMLElement).classList.contains("fa-paperclip") ||
      (e.target as HTMLElement).tagName === "BUTTON" ||
      (e.target as HTMLElement).tagName === "INPUT" ||
      (e.target as HTMLElement).closest("button")
    ) {
      return;
    }

    // Toggle expanded state to show/hide items
    setExpanded(!expanded);
  };

  // Get the items count
  const itemCount = sourceItems.length;

  return (
    <div
      className={`source-item ${source.active ? "active" : ""}`}
      data-tooltip={source.path}
      data-source-id={source.path}
      onClick={handleSourceClick}
    >
      <div className="source-item-header">
        <span className="source-icon">
          <i className="fa-solid fa-book"></i>
        </span>
        <span
          className="source-item-name"
          onClick={() => setExpanded(!expanded)}
        >
          {source.path}
        </span>
        <span className="number-of-elements">
          ({itemCount} {itemCount === 1 ? "item" : "items"})
        </span>
        <label className="switch">
          <input
            type="checkbox"
            className="source-toggle"
            checked={source.active}
            onChange={(e: ChangeEvent<HTMLInputElement>) =>
              onToggle(e.target.checked)
            }
          />
          <span className="slider"></span>
        </label>
        <span
          className="upload-icon upload-button"
          title="Upload file to source"
          onClick={handleAttachClick}
        >
          <i className="fa-solid fa-paperclip"></i>
        </span>
        {isUploading && <span className="upload-status">Uploading...</span>}
        <div className="source-actions">
          <button
            className="source-action-btn delete-source"
            title="Delete"
            onClick={handleDeleteClick}
          >
            <i className="fas fa-trash"></i>
          </button>
        </div>
      </div>

      <div
        className="source-item-list"
        style={{ display: expanded ? "block" : "none" }}
      >
        <ul>
          {sourceItems.map((item, index) => (
            <li key={index}>{item.uri}</li>
          ))}
        </ul>
      </div>

      <input
        ref={fileInputRef}
        type="file"
        style={{ display: "none" }}
        onChange={handleFileSelect}
      />
    </div>
  );
};

export default SourcesSection;
