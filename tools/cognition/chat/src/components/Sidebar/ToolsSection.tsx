import React, { useState, MouseEvent, KeyboardEvent, ChangeEvent } from "react";
import { useConfig } from "../../contexts/ConfigContext";
import "./ToolsSection.css";

interface Tool {
  id: string;
  actor: string;
  active: boolean;
}

interface ToolItemProps {
  tool: Tool;
  onToggle: (isActive: boolean) => void;
  onDelete: () => void;
}

const ToolsSection: React.FC = () => {
  const {
    toolsDetailsOpen,
    setToolsDetailsOpen,
    savedTools,
    addNewTool,
    setToolState,
    deleteTool,
    deleteAllTools,
  } = useConfig();

  const [toolInput, setToolInput] = useState<string>("");

  const handleAddTool = (): void => {
    if (toolInput.trim()) {
      const success = addNewTool(toolInput);
      if (success) {
        setToolInput("");
      } else {
        alert("This tool already exists!");
      }
    }
  };

  const handleToolToggle = (toolId: string, isActive: boolean): void => {
    setToolState(toolId, isActive);
  };

  return (
    <details
      id="tools-details"
      className="details details-transition-fix"
      open={toolsDetailsOpen}
      onToggle={(e) =>
        setToolsDetailsOpen((e.target as HTMLDetailsElement).open)
      }
    >
      <summary>
        <i className="fa-solid fa-wrench" style={{ marginRight: "8px" }}></i>{" "}
        Tools
      </summary>

      <div className="section tools-section">
        <details className="input-detail">
          <summary className="input-summary">
            <h4>
              <i className="fas fa-plus-circle"></i> Add Agent
            </h4>
          </summary>

          <div className="source-content">
            <div className="tools-form" id="tool-form">
              <div className="form-group">
                <input
                  type="text"
                  placeholder="Agent name"
                  value={toolInput}
                  onChange={(e: ChangeEvent<HTMLInputElement>) =>
                    setToolInput(e.target.value)
                  }
                  onKeyPress={(e: KeyboardEvent<HTMLInputElement>) =>
                    e.key === "Enter" && handleAddTool()
                  }
                />
              </div>
            </div>
            <div className="form-actions">
              <button
                id="save-tool-btn"
                className="action-button"
                onClick={handleAddTool}
              >
                Save
              </button>
            </div>
          </div>
        </details>

        <div className="saved-tools-section">
          <div className="available-header">
            <h3
              className="section-inner-title"
              style={{ marginBottom: "10px" }}
            >
              Available Tools
            </h3>
            {savedTools.length > 0 && (
              <button
                id="delete-all-tools"
                className="delete-all-btn"
                onClick={deleteAllTools}
                title="Delete all tools"
              >
                Delete all
              </button>
            )}
          </div>

          <div id="saved-tools-list" className="tools-list">
            {savedTools.map((tool) => (
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

const ToolItem: React.FC<ToolItemProps> = ({ tool, onToggle, onDelete }) => {
  return (
    <div
      className={`tool-item ${tool.active ? "active" : ""}`}
      data-tool-id={tool.id}
    >
      <div className="tool-item-header">
        <span className="tool-icon">
          <i className="fas fa-wrench"></i>
        </span>
        <span className="tool-item-name">{tool.actor}</span>
        <label className="switch">
          <input
            type="checkbox"
            className="tool-toggle"
            checked={tool.active}
            onChange={(e: ChangeEvent<HTMLInputElement>) =>
              onToggle(e.target.checked)
            }
          />
          <span className="slider"></span>
        </label>
        <div className="tool-actions">
          <button
            className="tool-action-btn delete-tool"
            title="Delete"
            onClick={(e: MouseEvent<HTMLButtonElement>) => {
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
